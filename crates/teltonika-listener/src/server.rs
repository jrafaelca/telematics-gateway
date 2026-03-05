//! Per-connection TCP handler for the Teltonika protocol.
//!
//! [`handle_connection`] runs in its own Tokio task for every accepted socket.
//! It performs the IMEI handshake, then loops reading framed AVL packets until
//! the device closes the connection or an unrecoverable read error occurs.
//!
//! # Connection flow
//!
//! 1. **IMEI handshake**
//!    a. Read 2-byte u16 BE length.
//!    b. Read `length` bytes of ASCII IMEI digits.
//!    c. Send `0x01` (accept) or `0x00` (reject).
//!
//! 2. **Packet loop** (repeated until EOF)
//!    a. Read 4-byte preamble (expected `0x00000000`).
//!    b. Read 4-byte u32 BE `data_field_length`.
//!    c. Read `data_field_length` bytes (data field).
//!    d. Read 4-byte u32 BE CRC.
//!    e. Validate CRC-16/IBM over the data field; log WARN and drop connection on mismatch.
//!    f. Parse AVL packet (codec ID + records).
//!    g. Send 4-byte u32 BE response = number of accepted records.
//!    h. Normalize and publish each GPS record (fire-and-forget).
//!    i. Update presence counters.
//!    j. Deliver pending server→device commands (Codec 12).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

use crate::commands;
use crate::crc::crc16_ibm;
use crate::normalize;
use crate::presence;
use crate::protocol::parse_packet;
use shared::publisher::Publisher;

/// Handles a single device connection until EOF or error.
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

    // ── 1. IMEI handshake ────────────────────────────────────────────────────

    let imei = match perform_imei_handshake(&mut socket, addr).await {
        Some(imei) => imei,
        None => {
            tracing::warn!(peer = %addr, "IMEI handshake failed, closing connection");
            return;
        }
    };

    presence::on_connect(imei, addr, connected_at, &mut redis_conn).await;

    // ── 2. Packet loop ───────────────────────────────────────────────────────

    loop {
        // a. Read 4-byte preamble.
        let mut preamble = [0u8; 4];
        match timeout(IDLE_TIMEOUT, socket.read_exact(&mut preamble)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Ok(Err(e)) => {
                tracing::warn!(peer = %addr, error = %e, "read error (preamble)");
                break;
            }
            Err(_) => {
                tracing::info!(peer = %addr, "idle timeout, closing");
                break;
            }
        }
        if preamble != [0x00, 0x00, 0x00, 0x00] {
            tracing::warn!(
                peer = %addr,
                preamble = ?preamble,
                "unexpected preamble, closing connection"
            );
            break;
        }

        // b. Read 4-byte data field length.
        let mut dfl_buf = [0u8; 4];
        if let Err(e) = socket.read_exact(&mut dfl_buf).await {
            tracing::warn!(peer = %addr, error = %e, "read error (data_field_length)");
            break;
        }
        let dfl = u32::from_be_bytes(dfl_buf) as usize;

        // c. Read data field.
        let mut data_field = vec![0u8; dfl];
        if let Err(e) = socket.read_exact(&mut data_field).await {
            tracing::warn!(peer = %addr, error = %e, "read error (data field)");
            break;
        }

        let received_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // d. Read 4-byte CRC.
        let mut crc_buf = [0u8; 4];
        if let Err(e) = socket.read_exact(&mut crc_buf).await {
            tracing::warn!(peer = %addr, error = %e, "read error (CRC)");
            break;
        }
        let crc_recv = u32::from_be_bytes(crc_buf) as u16;

        // e. Validate CRC.
        let crc_calc = crc16_ibm(&data_field);
        if crc_recv != crc_calc {
            tracing::warn!(
                peer = %addr,
                imei,
                dfl,
                crc_recv = format_args!("0x{:04X}", crc_recv),
                crc_calc = format_args!("0x{:04X}", crc_calc),
                "CRC mismatch, responding with 0 (device will retransmit)"
            );
            send_response(&mut socket, 0).await;
            continue;
        }

        // f. Parse AVL packet.
        let packet = match parse_packet(&data_field) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(peer = %addr, imei, error = %e, "parse error");
                // Respond with 0 accepted records so device retransmits.
                send_response(&mut socket, 0).await;
                continue;
            }
        };

        let num_records = packet.records.len();

        tracing::info!(
            peer = %addr,
            imei,
            codec = format_args!("0x{:02X}", packet.codec_id),
            records = num_records,
            "AVL packet received"
        );
        for r in &packet.records {
            tracing::debug!(
                imei,
                ts_ms = r.timestamp_ms,
                priority = r.priority,
                lat = format_args!("{:.6}", r.latitude),
                lon = format_args!("{:.6}", r.longitude),
                sat = r.satellites,
                speed = r.speed,
                alt = r.altitude,
                event_io = r.event_io_id,
                io_count = r.io_elements.len(),
                "AVL record"
            );
        }

        // g. Send response = number of accepted records.
        send_response(&mut socket, num_records as u32).await;

        // h. Normalize and publish each GPS record (fire-and-forget).
        let mut published = 0usize;
        for record in &packet.records {
            if let Some(normalized) = normalize::normalize(imei, record, received_at) {
                let pub_clone = publisher.clone();
                tokio::spawn(async move { pub_clone.publish(&normalized).await });
                published += 1;
            }
        }

        // i. Update presence.
        presence::on_packet(imei, published, &mut redis_conn).await;

        // j. Deliver pending server→device commands while the device listens.
        let delivered =
            commands::deliver_pending_commands(&mut socket, addr, imei, &mut redis_conn).await;

        if delivered > 0 {
            presence::on_commands_delivered(imei, delivered, &mut redis_conn).await;
        }
    }

    presence::on_disconnect(imei, connected_at, &mut redis_conn).await;
    tracing::info!(peer = %addr, "device disconnected");
}

/// Performs the IMEI handshake: reads the 2-byte length + IMEI bytes, sends
/// `0x01` on success or `0x00` on failure.  Returns the parsed IMEI or `None`.
async fn perform_imei_handshake(socket: &mut TcpStream, addr: SocketAddr) -> Option<u64> {
    // Read 2-byte u16 BE length.
    let mut len_buf = [0u8; 2];
    match timeout(IDLE_TIMEOUT, socket.read_exact(&mut len_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            tracing::warn!(peer = %addr, error = %e, "read error (IMEI length)");
            return None;
        }
        Err(_) => {
            tracing::warn!(peer = %addr, "timeout waiting for IMEI");
            return None;
        }
    }
    let imei_len = u16::from_be_bytes(len_buf) as usize;
    if imei_len == 0 || imei_len > 20 {
        tracing::warn!(peer = %addr, imei_len, "invalid IMEI length");
        let _ = socket.write_all(&[0x00]).await;
        return None;
    }

    // Read IMEI bytes.
    let mut imei_buf = vec![0u8; imei_len];
    if let Err(e) = socket.read_exact(&mut imei_buf).await {
        tracing::warn!(peer = %addr, error = %e, "read error (IMEI bytes)");
        return None;
    }

    // Parse ASCII IMEI.
    let imei_str = match std::str::from_utf8(&imei_buf) {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!(peer = %addr, "IMEI is not valid UTF-8");
            let _ = socket.write_all(&[0x00]).await;
            return None;
        }
    };
    let imei: u64 = match imei_str.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!(peer = %addr, imei_str, "IMEI parse failed");
            let _ = socket.write_all(&[0x00]).await;
            return None;
        }
    };

    // Accept the device.
    if let Err(e) = socket.write_all(&[0x01]).await {
        tracing::warn!(peer = %addr, error = %e, "failed to send IMEI accept");
        return None;
    }

    tracing::info!(peer = %addr, imei, "IMEI accepted");
    Some(imei)
}

/// Sends a 4-byte big-endian response indicating the number of accepted records.
async fn send_response(socket: &mut TcpStream, count: u32) {
    let resp = count.to_be_bytes();
    if let Err(e) = socket.write_all(&resp).await {
        tracing::error!(error = %e, "response send failed");
    } else {
        tracing::debug!(count, "response sent");
    }
}
