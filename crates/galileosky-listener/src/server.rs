//! Per-connection TCP handler for the Galileosky protocol.
//!
//! [`handle_connection`] runs in its own Tokio task for every accepted socket.
//! It loops reading framed Galileosky packets until the device closes the
//! connection or an unrecoverable read error occurs.
//!
//! # Packet loop
//!
//! 1. Read 1-byte header (with `IDLE_TIMEOUT`).
//! 2. Read 2-byte LE length: extract `is_archive` flag and `tag_len`.
//! 3. Read `tag_len` bytes (tag section).
//! 4. Read 2-byte LE CRC.
//! 5. Validate CRC-16 MODBUS over header + length + tags; log WARN and retry on mismatch (no NACK).
//! 6. Parse tags; learn IMEI from tag 0x03 on first occurrence.
//! 7. Send 3-byte ACK: `[0x02, crc_lo, crc_hi]`.
//! 8. Normalise and publish GPS record (if coordinates are present and valid).
//! 9. Deliver pending server→device commands.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

use crate::commands;
use crate::crc::crc16_modbus;
use crate::normalize;
use crate::presence;
use crate::protocol::parse_packet;
use shared::publisher::Publisher;

/// Handles a single device connection until EOF or error.
///
/// Publishing is fire-and-forget: each GPS record is spawned as an independent
/// task so a slow Valkey write cannot stall the read loop.
///
/// After ACKing each device packet, pending server→device commands stored in
/// `commands:{imei}` are delivered via [`commands::deliver_pending_commands`].
pub async fn handle_connection(
    mut socket: TcpStream,
    addr: SocketAddr,
    publisher: Arc<Publisher>,
    mut redis_conn: redis::aio::ConnectionManager,
) {
    tracing::info!(peer = %addr, "device connected");

    let connected_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // IMEI is learned from the first packet that contains tag 0x03.
    let mut known_imei: Option<u64> = None;

    loop {
        // 1. Read the 1-byte header.
        let mut hdr_buf = [0u8; 1];
        match timeout(IDLE_TIMEOUT, socket.read_exact(&mut hdr_buf)).await {
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
        let header_byte = hdr_buf[0];

        // 2. Read the 2-byte LE length field.
        let mut len_buf = [0u8; 2];
        if let Err(e) = socket.read_exact(&mut len_buf).await {
            tracing::warn!(peer = %addr, error = %e, "read error (length)");
            break;
        }
        let raw_len = u16::from_le_bytes(len_buf);
        let tag_len = (raw_len & 0x7FFF) as usize;

        // 3. Read the tag section.
        let mut tag_buf = vec![0u8; tag_len];
        if let Err(e) = socket.read_exact(&mut tag_buf).await {
            tracing::warn!(peer = %addr, error = %e, "read error (tags)");
            break;
        }

        // Capture receive time before any processing.
        let received_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // 4. Read the 2-byte LE CRC.
        let mut crc_buf = [0u8; 2];
        if let Err(e) = socket.read_exact(&mut crc_buf).await {
            tracing::warn!(peer = %addr, error = %e, "read error (CRC)");
            break;
        }
        let crc_recv = u16::from_le_bytes(crc_buf);

        // 5. Validate CRC over header + length + tags.
        let mut frame = vec![header_byte, len_buf[0], len_buf[1]];
        frame.extend_from_slice(&tag_buf);
        let crc_calc = crc16_modbus(&frame);

        if crc_recv != crc_calc {
            tracing::warn!(
                peer = %addr,
                tag_len,
                crc_recv = format_args!("0x{:04X}", crc_recv),
                crc_calc = format_args!("0x{:04X}", crc_calc),
                "CRC mismatch, ignoring packet (device will retransmit)"
            );
            continue;
        }

        // 6. Parse tags.
        let packet = match parse_packet(&frame) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(peer = %addr, tag_len, error = %e, "parse error");
                // Still ACK so the device doesn't retransmit indefinitely.
                send_ack(&mut socket, crc_recv).await;
                continue;
            }
        };

        // Learn IMEI from tag 0x03 (present at least in the head packet).
        if let Some(imei) = packet.tags.imei {
            if known_imei.is_none() {
                known_imei = Some(imei);
                presence::on_connect(imei, addr, connected_at, &mut redis_conn).await;
            }
        }

        log_packet(&packet, tag_len, addr, known_imei);

        // 7. Send ACK.
        send_ack(&mut socket, crc_recv).await;

        if let Some(imei) = known_imei {
            // 8. Normalise and publish (fire-and-forget).
            let num_records = if let Some(normalized) =
                normalize::normalize(imei, &packet.tags, received_at)
            {
                let pub_clone = publisher.clone();
                tokio::spawn(async move { pub_clone.publish(&normalized).await });
                1
            } else {
                0
            };

            presence::on_packet(imei, num_records, &mut redis_conn).await;

            // 9. Deliver pending commands while the device is still listening.
            let delivered =
                commands::deliver_pending_commands(&mut socket, addr, imei, &mut redis_conn)
                    .await;

            if delivered > 0 {
                presence::on_commands_delivered(imei, delivered, &mut redis_conn).await;
            }
        } else {
            tracing::warn!(peer = %addr, "packet received but IMEI not yet known, skipping publish");
        }
    }

    if let Some(imei) = known_imei {
        presence::on_disconnect(imei, connected_at, &mut redis_conn).await;
    }

    tracing::info!(peer = %addr, "device disconnected");
}

/// Sends a 3-byte Galileosky ACK: `[0x02, crc_lo, crc_hi]`.
///
/// The checksum echoed back is the CRC of the received packet, allowing the
/// device to verify the server received its packet correctly.
pub async fn send_ack(socket: &mut TcpStream, received_crc: u16) {
    let ack = [0x02u8, (received_crc & 0xFF) as u8, (received_crc >> 8) as u8];
    if let Err(e) = socket.write_all(&ack).await {
        tracing::error!(error = %e, "ACK send failed");
    } else {
        tracing::debug!(
            crc = format_args!("0x{:04X}", received_crc),
            "ACK sent"
        );
    }
}

/// Logs a human-readable summary of a received packet.
fn log_packet(packet: &crate::protocol::Packet, tag_len: usize, addr: SocketAddr, known_imei: Option<u64>) {
    let imei = packet.tags.imei.or(known_imei).unwrap_or(0);
    tracing::info!(
        peer = %addr,
        bytes = tag_len,
        imei,
        header = format_args!("0x{:02X}", packet.header),
        is_archive = packet.is_archive,
        has_gps = packet.tags.coordinates.is_some(),
        has_cmd = packet.tags.command_number.is_some(),
        "packet received"
    );
    if let Some(ref coords) = packet.tags.coordinates {
        tracing::debug!(
            ts = packet.tags.timestamp.unwrap_or(0),
            lat = format_args!("{:.6}", coords.latitude),
            lon = format_args!("{:.6}", coords.longitude),
            sat = coords.satellites,
            corr = coords.correctness,
            spd = packet.tags.speed_direction.as_ref().map(|s| s.speed_kmh).unwrap_or(0),
            alt = packet.tags.altitude.unwrap_or(0),
            hdop = packet.tags.hdop.unwrap_or(0),
            "GPS record"
        );
    }
}
