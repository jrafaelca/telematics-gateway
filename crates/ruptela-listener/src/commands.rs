//! Server-initiated command delivery to connected devices.
//!
//! After the server ACKs each device packet, [`deliver_pending_commands`] checks
//! `commands:{imei}` in Redis for pending entries and delivers them
//! while the device is still listening on the same TCP connection.
//!
//! # Redis model
//!
//! Key: `commands:{imei}` (Hash)
//! Field: a UUID string
//! Value: JSON — `{"cmd_text": "text", "status": "pending"}`
//!
//! After successful delivery the status field is updated to `"delivered"`.
//!
//! # Ruptela server→device framing
//!
//! Commands are sent as SMS via GPRS (command ID `0x6C`, 108), which is the
//! standard mechanism for sending text commands to Ruptela devices over TCP.
//!
//! ```text
//! [2B: packet_len (BE)] [1B: 0x6C] [cmd_text bytes] [2B: CRC16]
//! ```
//!
//! CRC16 is computed over `command_id + payload` (Kermit 0x8408).
//! The device acknowledges with command ID `0x07` (7).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use redis::AsyncCommands;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::crc::crc16;

/// Timeout for a single read/write exchange with the device during command delivery.
const CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Checks Redis for pending commands for `imei` and delivers each one over `socket`.
///
/// Uses `HLEN` as a fast-path guard (O(1)); only fetches the full hash when there
/// is at least one entry. Marks successfully delivered commands as `"delivered"`.
/// Returns the number of commands successfully delivered to the device.
pub async fn deliver_pending_commands(
    socket: &mut TcpStream,
    addr: SocketAddr,
    imei: u64,
    conn: &mut redis::aio::ConnectionManager,
) -> u32 {
    let key = format!("commands:{imei}");

    // Fast path: skip HGETALL if the hash is empty.
    let count: usize = match conn.hlen(&key).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(peer = %addr, imei, error = %e, "HLEN failed");
            return 0;
        }
    };

    if count == 0 {
        return 0;
    }

    tracing::info!(peer = %addr, imei, pending = count, "delivering pending commands");

    let all: HashMap<String, String> = match conn.hgetall(&key).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(peer = %addr, imei, error = %e, "HGETALL failed");
            return 0;
        }
    };

    let mut delivered_count = 0u32;

    for (uuid, json_str) in all {
        let val: Value = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "invalid JSON in command entry");
                continue;
            }
        };

        if val.get("status").and_then(|s| s.as_str()) != Some("pending") {
            continue;
        }

        let cmd_text = match val.get("cmd_text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                tracing::warn!(peer = %addr, uuid = %uuid, "missing cmd_text");
                continue;
            }
        };

        let frame = build_frame(&cmd_text);
        if let Err(e) = socket.write_all(&frame).await {
            tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command delivery failed");
            continue;
        }

        match read_device_ack(socket).await {
            Ok(true) => {
                let mut delivered = val.clone();
                if let Some(obj) = delivered.as_object_mut() {
                    obj.insert("status".to_string(), Value::String("delivered".to_string()));
                }
                match serde_json::to_string(&delivered) {
                    Ok(delivered_json) => {
                        let result: redis::RedisResult<()> =
                            conn.hset(&key, &uuid, &delivered_json).await;
                        if let Err(e) = result {
                            tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "HSET delivered failed");
                        } else {
                            tracing::info!(peer = %addr, uuid = %uuid, cmd = %cmd_text, "command delivered");
                            delivered_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "JSON serialize failed");
                    }
                }
            }
            Ok(false) => {
                tracing::warn!(peer = %addr, uuid = %uuid, "device responded negatively to command");
            }
            Err(e) => {
                tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command delivery failed");
            }
        }
    }

    delivered_count
}

/// Builds a Ruptela server→device frame for SMS via GPRS (cmd_id `0x6C`).
///
/// ```text
/// [2B: packet_len (BE)] [1B: 0x6C] [text bytes] [2B: CRC16]
/// ```
/// CRC16 covers `cmd_id + text` (Kermit 0x8408).
fn build_frame(cmd_text: &str) -> Vec<u8> {
    let text_bytes = cmd_text.as_bytes();

    let mut body = Vec::with_capacity(1 + text_bytes.len());
    body.push(0x6Cu8); // cmd_id = SMS via GPRS
    body.extend_from_slice(text_bytes);

    let packet_len = body.len() as u16;
    let crc = crc16(&body);

    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&packet_len.to_be_bytes());
    frame.extend_from_slice(&body);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// Reads the device's ACK response to a server-sent command.
///
/// Returns `Ok(true)` on command ID `0x07` (SMS ACK), `Ok(false)` on a
/// negative or unexpected response, and `Err` on I/O or timeout errors.
async fn read_device_ack(socket: &mut TcpStream) -> std::io::Result<bool> {
    let mut len_buf = [0u8; 2];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut len_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout waiting for device ACK"))??;

    let packet_len = u16::from_be_bytes(len_buf) as usize;
    if packet_len == 0 {
        return Ok(false);
    }

    let mut buf = vec![0u8; packet_len + 2];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading device ACK body"))??;

    let body = &buf[..packet_len];
    let crc_recv = u16::from_be_bytes([buf[packet_len], buf[packet_len + 1]]);
    let crc_calc = crc16(body);

    if crc_recv != crc_calc {
        tracing::warn!(
            crc_recv = format_args!("0x{:04X}", crc_recv),
            crc_calc = format_args!("0x{:04X}", crc_calc),
            "CRC mismatch in device ACK"
        );
        return Ok(false);
    }

    if body.is_empty() {
        return Ok(false);
    }

    // Command ID 0x07 = SMS via GPRS ACK.
    Ok(body[0] == 0x07)
}
