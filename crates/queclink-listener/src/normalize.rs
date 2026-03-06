//! Converts a Queclink [`FriRecord`] into a [`NormalizedRecord`].

use shared::normalize::NormalizedRecord;
use crate::protocol::FriRecord;

/// Converts a Queclink GPS record into the canonical [`NormalizedRecord`].
///
/// Returns `None` when:
/// - `gnss_accuracy == 0` (device reported no fix)
/// - coordinates are exactly `(0.0, 0.0)` (null island guard)
pub fn normalize(imei: u64, rec: &FriRecord, received_at: u64) -> Option<NormalizedRecord> {
    if rec.gnss_accuracy == 0.0 {
        return None;
    }
    if rec.longitude == 0.0 && rec.latitude == 0.0 {
        return None;
    }

    Some(NormalizedRecord {
        imei,
        received_at,
        timestamp: rec.timestamp,
        longitude: rec.longitude,
        latitude: rec.latitude,
        altitude: rec.altitude as f32,
        angle: rec.azimuth as f32,
        satellites: rec.satellites,
        speed: rec.speed.round() as u16,
        hdop: rec.hdop as f32,
        can_data: serde_json::json!({}),
    })
}
