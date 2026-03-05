//! Server-initiated command delivery to connected Teltonika devices.
//!
//! After the server responds to each AVL packet, [`deliver_pending_commands`]
//! checks `commands:{imei}` in Redis for pending entries and delivers them
//! while the device is still listening on the same TCP connection.
//!
//! # Redis model
//!
//! Key: `commands:{imei}` (Hash)
//! Field: a UUID string
//! Value: JSON — `{"cmd_text": "getinfo", "status": "pending"}`
//!
//! After successful delivery the status field is updated to `"delivered"`.
//!
//! # Teltonika server→device framing (Codec 12)
//!
//! ```text
//! [0x00000000]           preamble (4 B)
//! [dfl BE u32]           data_field_length (4 B) — from codec_id to qty2 inclusive
//! [0x0C]                 codec_id = Codec 12
//! [0x01]                 command quantity 1 (always 1)
//! [0x05]                 type = command
//! [cmd_len BE u32]       command text length
//! [cmd_len B]            command text (UTF-8, e.g. "getinfo")
//! [0x01]                 command quantity 2 (must match qty1)
//! [CRC-16/IBM BE u32]    CRC over data field (codec_id through qty2)
//! ```
//!
//! # Device response framing (Codec 12)
//!
//! Same structure with type = `0x06` (response) and the response text.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use redis::AsyncCommands;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::crc::crc16_ibm;

const CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Checks Redis for pending commands for `imei` and delivers each one over `socket`.
///
/// Uses `HLEN` as a fast-path guard (O(1)); only fetches the full hash when
/// there is at least one entry.  Marks successfully delivered commands as
/// `"delivered"`.  Returns the number of commands successfully delivered.
pub async fn deliver_pending_commands(
    socket: &mut TcpStream,
    addr: SocketAddr,
    imei: u64,
    conn: &mut redis::aio::ConnectionManager,
) -> u32 {
    let key = format!("commands:{imei}");

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

        let frame = build_command_frame(&cmd_text);

        if let Err(e) = socket.write_all(&frame).await {
            tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command send failed");
            continue;
        }

        match read_device_response(socket).await {
            Ok(Some(response_text)) => {
                let mut delivered = val.clone();
                if let Some(obj) = delivered.as_object_mut() {
                    obj.insert("status".to_string(), Value::String("delivered".to_string()));
                    obj.insert("response".to_string(), Value::String(response_text.clone()));
                }
                match serde_json::to_string(&delivered) {
                    Ok(delivered_json) => {
                        let result: redis::RedisResult<()> =
                            conn.hset(&key, &uuid, &delivered_json).await;
                        if let Err(e) = result {
                            tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "HSET delivered failed");
                        } else {
                            tracing::info!(
                                peer = %addr,
                                uuid = %uuid,
                                cmd = %cmd_text,
                                response = %response_text,
                                "command delivered"
                            );
                            delivered_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "JSON serialize failed");
                    }
                }
            }
            Ok(None) => {
                tracing::warn!(peer = %addr, uuid = %uuid, "device did not send a valid response");
            }
            Err(e) => {
                tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command delivery failed");
            }
        }
    }

    delivered_count
}

/// Builds a Codec 12 server→device command packet.
///
/// ```text
/// [preamble 4B][dfl BE 4B][0x0C][qty1=1][type=0x05][cmd_len BE 4B][cmd bytes][qty2=1][CRC BE 4B]
/// ```
fn build_command_frame(cmd_text: &str) -> Vec<u8> {
    let text_bytes = cmd_text.as_bytes();
    let cmd_len = text_bytes.len() as u32;

    // Data field: codec_id + qty1 + type + cmd_len + cmd + qty2.
    let mut data_field = Vec::new();
    data_field.push(0x0Cu8);             // codec_id = Codec 12
    data_field.push(0x01u8);             // command quantity 1
    data_field.push(0x05u8);             // type = command
    data_field.extend_from_slice(&cmd_len.to_be_bytes());
    data_field.extend_from_slice(text_bytes);
    data_field.push(0x01u8);             // command quantity 2

    let dfl = data_field.len() as u32;
    let crc = crc16_ibm(&data_field) as u32;

    let mut frame = vec![0x00u8, 0x00, 0x00, 0x00]; // preamble
    frame.extend_from_slice(&dfl.to_be_bytes());
    frame.extend_from_slice(&data_field);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// Reads a Codec 12 response packet from the device.
///
/// Returns `Ok(Some(text))` on a valid type-0x06 response, `Ok(None)` on CRC
/// mismatch or unexpected type, and `Err` on I/O or timeout errors.
async fn read_device_response(socket: &mut TcpStream) -> std::io::Result<Option<String>> {
    // Preamble (4B).
    let mut preamble = [0u8; 4];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut preamble))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading response preamble"))??;

    // Data field length (4B BE).
    let mut dfl_buf = [0u8; 4];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut dfl_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading response dfl"))??;
    let dfl = u32::from_be_bytes(dfl_buf) as usize;

    // Data field.
    let mut data_field = vec![0u8; dfl];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut data_field))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading response data field"))??;

    // CRC (4B BE).
    let mut crc_buf = [0u8; 4];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut crc_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading response CRC"))??;
    let crc_recv = u32::from_be_bytes(crc_buf) as u16;

    let crc_calc = crc16_ibm(&data_field);
    if crc_recv != crc_calc {
        tracing::warn!(
            crc_recv = format_args!("0x{:04X}", crc_recv),
            crc_calc = format_args!("0x{:04X}", crc_calc),
            "CRC mismatch in command response"
        );
        return Ok(None);
    }

    // Parse data field: codec_id(1) + qty1(1) + type(1) + resp_len(4) + text + qty2(1).
    if data_field.len() < 7 {
        tracing::warn!(len = data_field.len(), "response data field too short");
        return Ok(None);
    }

    let codec_id = data_field[0];
    let msg_type = data_field[2];

    if codec_id != 0x0C {
        tracing::warn!(codec_id, "unexpected codec ID in response");
        return Ok(None);
    }
    if msg_type != 0x06 {
        tracing::warn!(msg_type, "expected response type 0x06");
        return Ok(None);
    }

    let resp_len = u32::from_be_bytes([data_field[3], data_field[4], data_field[5], data_field[6]]) as usize;
    if data_field.len() < 7 + resp_len {
        tracing::warn!("response text truncated");
        return Ok(None);
    }

    let text = String::from_utf8_lossy(&data_field[7..7 + resp_len]).into_owned();
    Ok(Some(text))
}
