use shared::normalize::NormalizedRecord;
use crate::protocol::Record;

/// Converts a raw Ruptela [`Record`] into a [`NormalizedRecord`].
///
/// Returns `None` when the record has no GPS fix (`satellites == 0`) or when
/// both longitude and latitude are zero (null-island position sent on no fix).
///
/// IO elements are serialised into `can_data` as a JSON object keyed by IO ID.
///
/// `received_at` is a Unix millisecond timestamp captured by the server at
/// packet-read time, used to track processing latency.
pub fn normalize(imei: u64, record: &Record, received_at: u64) -> Option<NormalizedRecord> {
    if record.satellites == 0 {
        return None;
    }

    if record.longitude == 0.0 && record.latitude == 0.0 {
        return None;
    }

    let mut io_map = serde_json::Map::new();
    for elem in &record.io {
        io_map.insert(elem.id.to_string(), serde_json::json!(elem.value));
    }

    Some(NormalizedRecord {
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
        can_data: serde_json::Value::Object(io_map),
    })
}
