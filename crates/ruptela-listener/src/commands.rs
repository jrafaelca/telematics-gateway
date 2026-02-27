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
//! Value: JSON — `{"cmd_id": 108, "payload": "text", "status": "pending"}`
//!
//! After successful delivery the status field is updated to `"delivered"`.
//!
//! # Ruptela server→device framing
//!
//! ```text
//! [2B: packet_len (BE)] [1B: command_id] [N B: payload] [2B: CRC16]
//! ```
//! CRC16 is computed over `command_id + payload` (Kermit 0x8408, same as
//! device→server packets). No IMEI is sent in server-originated frames.
//!
//! # Supported commands
//!
//! | cmd_id | Name          | payload format               | Device ACK cmd |
//! |--------|---------------|------------------------------|----------------|
//! | 0x6C (108) | SMS via GPRS | UTF-8 text, up to 160 chars | 0x07 (7)      |
//! | 0x75 (117) | Set IO Value | 4B IO_ID + 4B IO_value (BE) | 0x11 (17), byte 0 = success |
//! | other  | unknown       | raw bytes from `payload` field | not validated |
//!
//! For binary payloads (e.g. cmd 117) store the `payload` field in the JSON as
//! a JSON array of byte values: `[0,0,0,1, 0,0,0,1]`.

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

        let cmd_id = match val.get("cmd_id").and_then(|v| v.as_u64()) {
            Some(id) if id <= 255 => id as u8,
            _ => {
                tracing::warn!(peer = %addr, uuid = %uuid, "missing or invalid cmd_id");
                continue;
            }
        };

        let payload_bytes = extract_payload(&val);

        // Build and send the server→device frame.
        let frame = build_frame(cmd_id, &payload_bytes);
        if let Err(e) = socket.write_all(&frame).await {
            tracing::warn!(peer = %addr, uuid = %uuid, cmd_id, error = %e, "command delivery failed");
            continue;
        }

        // Read the device's ACK response.
        match read_device_ack(socket, cmd_id).await {
            Ok(true) => {
                // Update status to "delivered" in Redis.
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
                            tracing::info!(peer = %addr, uuid = %uuid, cmd_id, "command delivered");
                            delivered_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "JSON serialize failed");
                    }
                }
            }
            Ok(false) => {
                tracing::warn!(peer = %addr, uuid = %uuid, cmd_id, "device responded negatively to command");
            }
            Err(e) => {
                tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command delivery failed");
            }
        }
    }

    delivered_count
}

/// Extracts the raw payload bytes from the JSON command value.
///
/// - `Value::String` → UTF-8/ASCII bytes (for cmd 108 SMS text)
/// - `Value::Array`  → array of u8 values (for cmd 117 IO control or any binary payload)
/// - anything else   → empty
fn extract_payload(val: &Value) -> Vec<u8> {
    match val.get("payload") {
        Some(Value::String(s)) => s.as_bytes().to_vec(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_u64().map(|b| b as u8))
            .collect(),
        _ => vec![],
    }
}

/// Builds a Ruptela server→device frame.
///
/// ```text
/// [2B: packet_len (BE)] [1B: cmd_id] [payload bytes] [2B: CRC16]
/// ```
/// CRC16 covers `cmd_id + payload` (Kermit 0x8408).
fn build_frame(cmd_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + payload.len());
    body.push(cmd_id);
    body.extend_from_slice(payload);

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
/// Returns `Ok(true)` on a positive ACK, `Ok(false)` on a negative ACK or CRC
/// mismatch, and `Err` on I/O or timeout errors.
async fn read_device_ack(socket: &mut TcpStream, sent_cmd_id: u8) -> std::io::Result<bool> {
    // Read the 2-byte length header.
    let mut len_buf = [0u8; 2];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut len_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout waiting for device ACK"))??;

    let packet_len = u16::from_be_bytes(len_buf) as usize;
    if packet_len == 0 {
        return Ok(false);
    }

    // Read body + 2-byte CRC.
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

    let response_cmd = body[0];
    let ack_byte = body.get(1).copied();

    // Validate the response command ID matches what we expect.
    let expected = expected_ack_cmd(sent_cmd_id);
    if expected != 0 && response_cmd != expected {
        tracing::warn!(
            sent = format_args!("0x{:02X}", sent_cmd_id),
            response = format_args!("0x{:02X}", response_cmd),
            expected = format_args!("0x{:02X}", expected),
            "unexpected response command in device ACK"
        );
        return Ok(false);
    }

    // For cmd 117 (Set IO Value): ack byte 0 = changed (success).
    if sent_cmd_id == 0x75 {
        return Ok(ack_byte == Some(0));
    }

    Ok(true)
}

/// Returns the expected response command ID from the device for a given server command.
///
/// Returns 0 for unknown commands (response cmd is not validated).
fn expected_ack_cmd(sent_cmd_id: u8) -> u8 {
    match sent_cmd_id {
        0x6C => 0x07, // SMS via GPRS  → device replies with Cmd 7
        0x75 => 0x11, // Set IO Value  → device replies with Cmd 17
        _ => 0,       // unknown: skip response-cmd validation
    }
}
