//! Valkey/Redis stream publisher.
//!
//! [`Publisher`] holds a connection-manager (which handles reconnection
//! automatically) and exposes a single [`Publisher::publish`] method that
//! appends a [`NormalizedRecord`] to the appropriate shard stream via `XADD`.

use redis::AsyncCommands;
use crate::normalize::{NormalizedRecord, stream_key};

/// An async publisher backed by a Valkey/Redis connection manager.
///
/// The inner [`redis::aio::ConnectionManager`] is cheaply cloneable and
/// multiplexes commands over a single underlying connection, making it safe
/// to share across Tokio tasks via [`std::sync::Arc`].
pub struct Publisher {
    manager: redis::aio::ConnectionManager,
    num_shards: u64,
}

impl Publisher {
    /// Creates a new `Publisher` and establishes the initial connection.
    ///
    /// `redis_url` accepts any URL supported by the `redis` crate, e.g.
    /// `redis://127.0.0.1:6379` or `redis://:password@host:port/db`.
    pub async fn new(redis_url: &str, num_shards: u64) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let manager = redis::aio::ConnectionManager::new(client).await?;
        Ok(Self { manager, num_shards })
    }

    /// Returns a cloned [`redis::aio::ConnectionManager`] that can be used to
    /// issue arbitrary Redis commands (e.g. for command delivery in listeners).
    ///
    /// The clone is cheap — both share the same underlying multiplexed connection.
    pub fn connection_manager(&self) -> redis::aio::ConnectionManager {
        self.manager.clone()
    }

    /// Appends `record` to the stream `devices:records:{IMEI % shards}`.
    ///
    /// Errors are logged but not propagated; a failed publish does not affect
    /// the TCP connection or the ACK sent to the device.
    pub async fn publish(&self, record: &NormalizedRecord) {
        let key = stream_key(record.imei, self.num_shards);
        let fields = record.to_fields();

        let mut conn = self.manager.clone();
        let result: redis::RedisResult<String> = conn.xadd(&key, "*", fields.as_slice()).await;

        if let Err(e) = result {
            tracing::error!(stream = %key, error = %e, "XADD failed");
        }
    }
}
