use shared::normalize::NormalizedRecord;
use crate::protocol::Record;

/// Converts a raw Ruptela [`Record`] into a [`NormalizedRecord`].
///
/// `received_at` is a Unix millisecond timestamp captured by the server at
/// packet-read time, used to track processing latency.
pub fn normalize(imei: u64, record: &Record, received_at: u64) -> NormalizedRecord {
    NormalizedRecord {
        imei,
        received_at,
        timestamp: record.timestamp,
        longitude: record.longitude,
        latitude: record.latitude,
        altitude: record.altitude,
        angle: record.angle,
        satellites: record.satellites,
        speed: record.speed,
        hdop: record.hdop,
        can_data: serde_json::json!({}),
    }
}
