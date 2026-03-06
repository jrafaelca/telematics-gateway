//! Server-initiated command delivery to connected Queclink devices.
//!
//! After the server SACKs each device message, [`deliver_pending_commands`]
//! checks `commands:{imei}` in Redis for pending entries and delivers them
//! while the device is still listening on the same TCP connection.
//!
//! # Redis model
//!
//! Key: `commands:{imei}` (Hash)
//! Field: a UUID string
//! Value: JSON — `{"cmd_text": "AT+GTRTO=gv310lau,3,,,,,", "status": "pending"}`
//!
//! `cmd_text` is the full AT command **without** the serial number and `$`.
//! At delivery time `,{serial_num:04X}$\r\n` is appended.
//!
//! After successful delivery the status is updated to `"delivered"`.
//!
//! # Command exchange
//!
//! Server sends:  `AT+GTRTO=gv310lau,3,,,,,0001$\r\n`
//! Device replies: `+ACK:GTRTO,{version},{IMEI},{device_name},GPS,{serial_num},{send_time},{count}$\r\n`
//! Match on `serial_num` in field index 5.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::AsyncCommands;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Timeout for a single command exchange with the device.
const CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Checks Redis for pending commands and delivers each one over the connection.
///
/// Returns the number of commands successfully delivered.
pub async fn deliver_pending_commands(
    reader: &mut BufReader<TcpStream>,
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

        // Serial number: millis as u16, incremented per command to avoid collisions.
        let serial_num = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u16)
            .wrapping_add(delivered_count as u16);

        let cmd_line = format!("{cmd_text},{serial_num:04X}$\r\n");

        if let Err(e) = reader.get_mut().write_all(cmd_line.as_bytes()).await {
            tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command send failed");
            continue;
        }

        match read_device_ack(reader, serial_num).await {
            Ok(true) => {
                let mut delivered = val.clone();
                if let Some(obj) = delivered.as_object_mut() {
                    obj.insert("status".to_string(), Value::String("delivered".to_string()));
                    obj.insert("response".to_string(), Value::String(String::new()));
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
                                serial_num,
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
            Ok(false) => {
                tracing::warn!(peer = %addr, uuid = %uuid, serial_num, "device did not confirm command");
            }
            Err(e) => {
                tracing::warn!(peer = %addr, uuid = %uuid, error = %e, "command delivery failed");
            }
        }
    }

    delivered_count
}

/// Reads lines from the device until a matching `+ACK:GTRTO` is received.
///
/// Returns `Ok(true)` when the ACK's serial number matches `expected_serial`,
/// `Ok(false)` on mismatch, wrong message type, or EOF, and `Err` on I/O or
/// timeout errors.
async fn read_device_ack(
    reader: &mut BufReader<TcpStream>,
    expected_serial: u16,
) -> io::Result<bool> {
    let mut line = String::new();
    let n = timeout(CMD_TIMEOUT, reader.read_line(&mut line))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timeout waiting for ACK"))??;

    if n == 0 {
        return Ok(false); // EOF
    }

    // Strip \r\n and trailing $
    let line = line.trim_end_matches(['\r', '\n']).trim_end_matches('$');
    let fields: Vec<&str> = line.split(',').collect();

    // Expect +ACK:GTRTO with serial_num at index 5
    if fields.first().map(|s| s.starts_with("+ACK:GTRTO")).unwrap_or(false) {
        if let Some(sn_str) = fields.get(5) {
            if let Ok(sn) = u16::from_str_radix(sn_str, 16) {
                if sn == expected_serial {
                    return Ok(true);
                }
            }
        }
        tracing::warn!(
            expected = format_args!("{:04X}", expected_serial),
            got = fields.get(5).unwrap_or(&""),
            "serial number mismatch in device ACK"
        );
        return Ok(false);
    }

    tracing::debug!(line = %line, "unexpected line while waiting for command ACK");
    Ok(false)
}
