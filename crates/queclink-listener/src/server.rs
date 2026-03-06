//! Per-connection TCP handler for the Queclink @Track ASCII protocol.
//!
//! [`handle_connection`] runs in its own Tokio task for every accepted socket.
//! It loops reading ASCII lines until the device closes the connection or an
//! idle timeout occurs.
//!
//! # Message loop
//!
//! 1. `read_line` with `IDLE_TIMEOUT` — break on EOF or timeout.
//! 2. `parse_line` → [`Message`].
//! 3. Dispatch:
//!    - `FriReport | LocationReport` → learn IMEI; normalise + publish; send SACK; deliver commands.
//!    - `Heartbeat` → send `+SACK:GTHBD,...$\r\n`; update presence.
//!    - `CommandAck` → log only (already handled by the commands module).
//!    - `Unknown` → log debug, continue.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::commands;
use crate::normalize;
use crate::presence;
use crate::protocol::{parse_line, Message};
use shared::publisher::Publisher;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Handles a single Queclink device connection until EOF or idle timeout.
pub async fn handle_connection(
    socket: TcpStream,
    addr: SocketAddr,
    publisher: Arc<Publisher>,
    mut redis_conn: redis::aio::ConnectionManager,
) {
    tracing::info!(peer = %addr, "device connected");

    let connected_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let mut reader = BufReader::new(socket);
    let mut known_imei: Option<u64> = None;

    loop {
        let mut buf = String::new();
        match timeout(IDLE_TIMEOUT, reader.read_line(&mut buf)).await {
            Ok(Ok(0)) => break, // EOF
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::warn!(peer = %addr, error = %e, "read error");
                break;
            }
            Err(_) => {
                tracing::info!(peer = %addr, "idle timeout, closing");
                break;
            }
        }

        let received_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let msg = parse_line(&buf);

        match msg {
            Message::FriReport(ref rec) | Message::LocationReport(ref rec) => {
                let imei = rec.imei;

                if known_imei.is_none() {
                    known_imei = Some(imei);
                    presence::on_connect(imei, addr, connected_at, &mut redis_conn).await;
                }

                tracing::info!(
                    peer = %addr,
                    imei,
                    msg_type = %rec.msg_type,
                    lon = format_args!("{:.6}", rec.longitude),
                    lat = format_args!("{:.6}", rec.latitude),
                    gnss_accuracy = rec.gnss_accuracy,
                    speed = rec.speed,
                    "GPS report received"
                );

                send_sack(
                    reader.get_mut(),
                    &rec.msg_type,
                    &rec.version,
                    imei,
                    &rec.device_name,
                    &rec.count,
                )
                .await;

                let num_records =
                    if let Some(normalized) = normalize::normalize(imei, rec, received_at) {
                        let pub_clone = publisher.clone();
                        tokio::spawn(async move { pub_clone.publish(&normalized).await });
                        1
                    } else {
                        0
                    };

                presence::on_packet(imei, num_records, &mut redis_conn).await;

                let delivered =
                    commands::deliver_pending_commands(&mut reader, addr, imei, &mut redis_conn)
                        .await;

                if delivered > 0 {
                    presence::on_commands_delivered(imei, delivered, &mut redis_conn).await;
                }
            }

            Message::Heartbeat(ref hbd) => {
                let imei = hbd.imei;

                if known_imei.is_none() {
                    known_imei = Some(imei);
                    presence::on_connect(imei, addr, connected_at, &mut redis_conn).await;
                }

                tracing::debug!(peer = %addr, imei, "heartbeat received");

                send_sack(
                    reader.get_mut(),
                    "HBD",
                    &hbd.version,
                    imei,
                    &hbd.device_name,
                    &hbd.count,
                )
                .await;

                presence::on_packet(imei, 0, &mut redis_conn).await;
            }

            Message::CommandAck(ref ack) => {
                tracing::debug!(
                    peer = %addr,
                    imei = ack.imei,
                    cmd = %ack.msg_type,
                    serial = %ack.serial_num,
                    version = %ack.version,
                    device = %ack.device_name,
                    count = %ack.count,
                    "command ACK received"
                );
            }

            Message::Unknown => {
                tracing::debug!(peer = %addr, line = %buf.trim_end(), "unknown line, ignoring");
            }
        }
    }

    if let Some(imei) = known_imei {
        presence::on_disconnect(imei, connected_at, &mut redis_conn).await;
    }

    tracing::info!(peer = %addr, "device disconnected");
}

/// Sends a `+SACK:GT{msg_type},{version},{imei},{device_name},{count}$\r\n` reply.
async fn send_sack(
    socket: &mut TcpStream,
    msg_type: &str,
    version: &str,
    imei: u64,
    device_name: &str,
    count: &str,
) {
    let sack = format!("+SACK:GT{msg_type},{version},{imei:015},{device_name},{count}$\r\n");
    if let Err(e) = socket.write_all(sack.as_bytes()).await {
        tracing::error!(error = %e, "SACK send failed");
    } else {
        tracing::debug!(msg_type, count, "SACK sent");
    }
}
