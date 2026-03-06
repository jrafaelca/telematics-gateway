//! Device presence and session state tracking.
//!
//! Maintains a Redis hash `devices:{imei}` with connection metadata,
//! counters, and the current status of each device.
//!
//! # Hash fields
//!
//! | Field            | Type   | Description                              |
//! |------------------|--------|------------------------------------------|
//! | `status`         | string | `"connected"` or `"disconnected"`        |
//! | `peer`           | string | IP:port of the current/last connection   |
//! | `connected_at`   | u64 ms | Timestamp of the last connection         |
//! | `disconnected_at`| u64 ms | Timestamp of the last disconnection      |
//! | `last_seen_at`   | u64 ms | Timestamp of the last received packet    |
//! | `last_session_ms`| u64 ms | Duration of the last session             |
//! | `session_count`  | u64    | Total sessions (accumulated)             |
//! | `packets_total`  | u64    | Total packets received (accumulated)     |
//! | `records_total`  | u64    | Total GPS records received (accumulated) |
//! | `commands_total` | u64    | Total commands delivered (accumulated)   |

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

fn key(imei: u64) -> String {
    format!("devices:{imei}")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Called when the device IMEI is first known (after the first parsed message).
pub async fn on_connect(
    imei: u64,
    addr: SocketAddr,
    connected_at: u64,
    conn: &mut redis::aio::ConnectionManager,
) {
    let key = key(imei);
    let result: redis::RedisResult<redis::Value> = redis::pipe()
        .hset(&key, "status", "connected")
        .hset(&key, "peer", addr.to_string())
        .hset(&key, "connected_at", connected_at)
        .hincr(&key, "session_count", 1i64)
        .query_async(conn)
        .await;

    if let Err(e) = result {
        tracing::warn!(imei, error = %e, "presence on_connect failed");
    }
}

/// Called on every successfully parsed message from the device.
pub async fn on_packet(
    imei: u64,
    num_records: usize,
    conn: &mut redis::aio::ConnectionManager,
) {
    let key = key(imei);
    let ts = now_ms();
    let result: redis::RedisResult<redis::Value> = redis::pipe()
        .hset(&key, "last_seen_at", ts)
        .hincr(&key, "packets_total", 1i64)
        .hincr(&key, "records_total", num_records as i64)
        .query_async(conn)
        .await;

    if let Err(e) = result {
        tracing::warn!(imei, error = %e, "presence on_packet failed");
    }
}

/// Called after successfully delivering one or more commands to the device.
pub async fn on_commands_delivered(
    imei: u64,
    count: u32,
    conn: &mut redis::aio::ConnectionManager,
) {
    let key = key(imei);
    let result: redis::RedisResult<redis::Value> = redis::pipe()
        .hincr(&key, "commands_total", count as i64)
        .query_async(conn)
        .await;

    if let Err(e) = result {
        tracing::warn!(imei, error = %e, "presence on_commands_delivered failed");
    }
}

/// Called when the device disconnects (EOF or unrecoverable error).
pub async fn on_disconnect(
    imei: u64,
    connected_at: u64,
    conn: &mut redis::aio::ConnectionManager,
) {
    let key = key(imei);
    let disconnected_at = now_ms();
    let session_ms = disconnected_at.saturating_sub(connected_at);

    let result: redis::RedisResult<redis::Value> = redis::pipe()
        .hset(&key, "status", "disconnected")
        .hset(&key, "disconnected_at", disconnected_at)
        .hset(&key, "last_session_ms", session_ms)
        .query_async(conn)
        .await;

    if let Err(e) = result {
        tracing::warn!(imei, error = %e, "presence on_disconnect failed");
    }
}
