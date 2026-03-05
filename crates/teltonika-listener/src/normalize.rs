use serde_json::json;
use shared::normalize::NormalizedRecord;

use crate::protocol::{AvlRecord, IoValue};

/// Converts a Teltonika [`AvlRecord`] into a [`NormalizedRecord`].
///
/// Returns `None` when the record has no GPS fix (`satellites == 0`) or when
/// both longitude and latitude are zero (device reported no position).
///
/// `imei` is provided by the caller (learned from the TCP IMEI handshake).
/// `received_at` is a Unix millisecond timestamp captured by the server at
/// packet-read time.
pub fn normalize(imei: u64, record: &AvlRecord, received_at: u64) -> Option<NormalizedRecord> {
    if record.satellites == 0 {
        return None;
    }

    // Reject the "null island" position sent when there is no GPS fix.
    if record.longitude == 0.0 && record.latitude == 0.0 {
        return None;
    }

    // Build can_data from IO elements.
    let mut io_map = serde_json::Map::new();
    for elem in &record.io_elements {
        let key = elem.id.to_string();
        let val = match &elem.value {
            IoValue::U8(v)    => json!(v),
            IoValue::U16(v)   => json!(v),
            IoValue::U32(v)   => json!(v),
            IoValue::U64(v)   => json!(v),
            IoValue::Bytes(b) => json!(hex::encode(b)),
        };
        io_map.insert(key, val);
    }

    Some(NormalizedRecord {
        imei,
        received_at,
        timestamp: (record.timestamp_ms / 1_000) as u32,
        longitude: record.longitude,
        latitude: record.latitude,
        altitude: record.altitude as f32,
        angle: record.angle as f32,
        satellites: record.satellites,
        speed: record.speed,
        hdop: 0.0,
        can_data: serde_json::Value::Object(io_map),
    })
}

/// Hex-encodes a byte slice (used for NX / variable-length IO values).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
