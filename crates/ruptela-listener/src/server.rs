//! Per-connection TCP handler.
//!
//! [`handle_connection`] runs in its own Tokio task for every accepted socket.
//! It loops reading framed Ruptela packets until the device closes the
//! connection or an unrecoverable read error occurs.
//!
//! # Packet loop
//!
//! 1. Read 2-byte `packet_len`.
//! 2. Read `packet_len` body bytes + 2 CRC bytes.
//! 3. Validate CRC16; send NACK and continue on mismatch.
//! 4. Parse the body with [`parse_packet`]; send NACK on error.
//! 5. Send ACK, normalise each record, publish to Valkey (fire-and-forget).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

use crate::crc::crc16;
use crate::normalize;
use crate::protocol::{Packet, Payload, parse_packet};
use shared::publisher::Publisher;

/// Handles a single device connection until EOF or error.
///
/// Publishing is fire-and-forget: each record is spawned as an independent
/// task so a slow Valkey write cannot stall the read loop.
pub async fn handle_connection(
    mut socket: TcpStream,
    addr: SocketAddr,
    publisher: Arc<Publisher>,
) {
    tracing::info!(peer = %addr, "device connected");

    loop {
        // 1. Read the 2-byte packet length.
        let mut len_buf = [0u8; 2];
        match timeout(IDLE_TIMEOUT, socket.read_exact(&mut len_buf)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Ok(Err(e)) => {
                tracing::warn!(peer = %addr, error = %e, "read error");
                break;
            }
            Err(_) => {
                tracing::info!(peer = %addr, "idle timeout, closing");
                break;
            }
        }

        let packet_len = u16::from_be_bytes(len_buf) as usize;

        // 2. Read body (packet_len bytes) + CRC (2 bytes).
        let total = packet_len + 2;
        let mut body_buf = vec![0u8; total];
        if let Err(e) = socket.read_exact(&mut body_buf).await {
            tracing::warn!(peer = %addr, error = %e, "read error");
            break;
        }

        // Capture receive time before any processing.
        let received_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // 3. Split body and CRC.
        let body = &body_buf[..packet_len];
        let crc_received = u16::from_be_bytes([body_buf[packet_len], body_buf[packet_len + 1]]);

        // 4. Validate CRC16.
        let crc_calculated = crc16(body);
        if crc_received != crc_calculated {
            tracing::warn!(
                peer = %addr,
                bytes = packet_len,
                crc_recv = format_args!("0x{:04X}", crc_received),
                crc_calc = format_args!("0x{:04X}", crc_calculated),
                "CRC mismatch"
            );
            send_nack(&mut socket).await;
            continue;
        }

        // 5. Parse and publish.
        match parse_packet(body) {
            Ok(packet) => {
                log_packet(&packet, packet_len, addr);
                send_ack(&mut socket, packet.command_id).await;

                let records = match &packet.payload {
                    Payload::Records { records, .. }
                    | Payload::ExtendedRecords { records, .. } => Some(records),
                    Payload::Unknown { .. } => None,
                };
                if let Some(records) = records {
                    for record in records {
                        let normalized = normalize::normalize(packet.imei, record, received_at);
                        let pub_clone = publisher.clone();
                        tokio::spawn(async move { pub_clone.publish(&normalized).await });
                    }
                }
            }
            Err(e) => {
                tracing::warn!(peer = %addr, bytes = packet_len, error = %e, "parse error");
                send_nack(&mut socket).await;
            }
        }
    }

    tracing::info!(peer = %addr, "device disconnected");
}

/// Sends an ACK frame for the given `command_id`.
///
/// The ACK body is `[command_id + 99, 0x01]`, framed with a 2-byte length
/// header and a 2-byte CRC16.
pub async fn send_ack(socket: &mut TcpStream, command_id: u8) {
    let response_cmd = command_id + 99; // e.g. cmd 0x01 → 0x64 (100)
    let body = [response_cmd, 0x01u8];
    let crc = crc16(&body);
    let mut response = vec![0x00, 0x02, response_cmd, 0x01];
    response.extend_from_slice(&crc.to_be_bytes());
    if let Err(e) = socket.write_all(&response).await {
        tracing::error!(error = %e, "ACK send failed");
    } else {
        tracing::debug!(cmd = format_args!("0x{:02X}", response_cmd), "ACK sent");
    }
}

/// Sends a NACK frame (`[0x64, 0x00]` with CRC).
pub async fn send_nack(socket: &mut TcpStream) {
    let body = [0x64u8, 0x00u8];
    let crc = crc16(&body);
    let mut response = vec![0x00, 0x02, 0x64, 0x00];
    response.extend_from_slice(&crc.to_be_bytes());
    if let Err(e) = socket.write_all(&response).await {
        tracing::error!(error = %e, "NACK send failed");
    } else {
        tracing::debug!("NACK sent");
    }
}

/// Logs a human-readable summary of a received packet.
fn log_packet(packet: &Packet, size: usize, addr: SocketAddr) {
    let cmd_label = match packet.command_id {
        0x01 => "Records",
        0x44 => "ExtendedRecords",
        _    => "Unknown",
    };
    tracing::info!(
        peer = %addr,
        bytes = size,
        imei = packet.imei,
        cmd = format_args!("0x{:02X}", packet.command_id),
        cmd_label,
        "packet received"
    );
    match &packet.payload {
        Payload::Records { records_left, num_records, records }
        | Payload::ExtendedRecords { records_left, num_records, records } => {
            tracing::debug!(records = num_records, left = records_left, "record batch");
            for (i, r) in records.iter().enumerate() {
                tracing::debug!(
                    index = i + 1,
                    ts = r.timestamp,
                    lon = format_args!("{:.6}", r.longitude),
                    lat = format_args!("{:.6}", r.latitude),
                    alt = format_args!("{:.1}", r.altitude),
                    spd = r.speed,
                    sat = r.satellites,
                    evt = r.event_id,
                    io_count = r.io.len(),
                    "record"
                );
            }
        }
        Payload::Unknown { command_id, raw } => {
            tracing::warn!(
                cmd = format_args!("0x{:02X}", command_id),
                bytes = raw.len(),
                "unknown command"
            );
        }
    }
}
