//! Server-initiated command delivery to connected Galileosky devices.
//!
//! After the server ACKs each device packet, [`deliver_pending_commands`] checks
//! `commands:{imei}` in Redis for pending entries and delivers them while the
//! device is still listening on the same TCP connection.
//!
//! # Redis model
//!
//! Key: `commands:{imei}` (Hash)
//! Field: a UUID string
//! Value: JSON — `{"cmd_text": "status", "status": "pending"}`
//!
//! After successful delivery the status field is updated to `"delivered"`.
//!
//! # Galileosky server→device framing
//!
//! A standard packet (header `0x01`) containing four tags:
//!
//! ```text
//! [0x01]                        ← header
//! [L_lo, L_hi]                  ← tag section length (LE)
//! [0x03][15 B IMEI ASCII]       ← tag 0x03: IMEI
//! [0x04][0x00, 0x00]            ← tag 0x04: device ID = 0 (any)
//! [0xE0][n0, n1, n2, n3]        ← tag 0xE0: command number (u32 LE)
//! [0xE1][len][text bytes]       ← tag 0xE1: command text (1B len prefix)
//! [crc_lo, crc_hi]              ← CRC-16 MODBUS (LE)
//! ```
//!
//! # Device reply
//!
//! The device responds with a standard packet containing tag 0xE0 (same
//! command number) and tag 0xE1 (reply text).  If the command number matches,
//! the command is marked as `"delivered"` in Redis.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::AsyncCommands;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::crc::crc16_modbus;
use crate::protocol::parse_packet;

/// Timeout for a single read/write exchange with the device during command delivery.
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

        // Generate a semi-unique command number using the current millisecond timestamp.
        let cmd_number = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u32)
            .wrapping_add(delivered_count);

        let frame = build_command_frame(imei, cmd_number, &cmd_text);

        if let Err(e) = socket.write_all(&frame).await {
            tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command send failed");
            continue;
        }

        match read_device_reply(socket, cmd_number).await {
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
                            tracing::info!(peer = %addr, uuid = %uuid, cmd_number, "command delivered");
                            delivered_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "JSON serialize failed");
                    }
                }
            }
            Ok(false) => {
                tracing::warn!(peer = %addr, uuid = %uuid, cmd_number, "device did not confirm command");
            }
            Err(e) => {
                tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command delivery failed");
            }
        }
    }

    delivered_count
}

/// Builds a Galileosky server→device command packet.
///
/// ```text
/// [0x01][L_lo L_hi][tag 0x03 IMEI][tag 0x04 device_id=0][tag 0xE0 cmd_num][tag 0xE1 text][crc_lo crc_hi]
/// ```
fn build_command_frame(imei: u64, cmd_number: u32, cmd_text: &str) -> Vec<u8> {
    let imei_str = format!("{imei:015}");
    let text_bytes = cmd_text.as_bytes();

    let mut tags = Vec::new();
    // tag 0x03: IMEI (15 bytes ASCII)
    tags.push(0x03u8);
    tags.extend_from_slice(imei_str.as_bytes());
    // tag 0x04: device ID = 0 (any)
    tags.extend_from_slice(&[0x04, 0x00, 0x00]);
    // tag 0xE0: command number (4B LE)
    tags.push(0xE0u8);
    tags.extend_from_slice(&cmd_number.to_le_bytes());
    // tag 0xE1: command text (1B length + bytes)
    tags.push(0xE1u8);
    tags.push(text_bytes.len() as u8);
    tags.extend_from_slice(text_bytes);

    let tag_len = tags.len() as u16;
    let mut frame = vec![0x01u8, (tag_len & 0xFF) as u8, (tag_len >> 8) as u8];
    frame.extend_from_slice(&tags);

    let crc = crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

/// Reads a Galileosky packet from the device and checks whether it confirms
/// the given `expected_cmd_number` via tag 0xE0.
///
/// Returns `Ok(true)` on confirmation, `Ok(false)` on CRC mismatch, wrong
/// command number, or parse failure, and `Err` on I/O or timeout errors.
async fn read_device_reply(
    socket: &mut TcpStream,
    expected_cmd_number: u32,
) -> std::io::Result<bool> {
    // Read header (1B).
    let mut hdr = [0u8; 1];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut hdr))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading reply header"))??;

    // Read 2-byte LE length.
    let mut len_buf = [0u8; 2];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut len_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading reply length"))??;

    let raw_len = u16::from_le_bytes(len_buf);
    let tag_len = (raw_len & 0x7FFF) as usize;

    // Read tag section.
    let mut tag_buf = vec![0u8; tag_len];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut tag_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading reply tags"))??;

    // Read 2-byte LE CRC.
    let mut crc_buf = [0u8; 2];
    timeout(CMD_TIMEOUT, socket.read_exact(&mut crc_buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout reading reply CRC"))??;

    let crc_recv = u16::from_le_bytes(crc_buf);

    // Validate CRC over header + len + tags.
    let mut frame = vec![hdr[0], len_buf[0], len_buf[1]];
    frame.extend_from_slice(&tag_buf);
    let crc_calc = crc16_modbus(&frame);

    if crc_recv != crc_calc {
        tracing::warn!(
            crc_recv = format_args!("0x{:04X}", crc_recv),
            crc_calc = format_args!("0x{:04X}", crc_calc),
            "CRC mismatch in device reply"
        );
        return Ok(false);
    }

    // Parse tags and check command number.
    match parse_packet(&frame) {
        Ok(packet) => {
            if packet.tags.command_number == Some(expected_cmd_number) {
                Ok(true)
            } else {
                tracing::warn!(
                    expected = expected_cmd_number,
                    got = ?packet.tags.command_number,
                    "command number mismatch in device reply"
                );
                Ok(false)
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse device reply");
            Ok(false)
        }
    }
}
