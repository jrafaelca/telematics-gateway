use shared::normalize::NormalizedRecord;
use crate::protocol::TagSet;

/// Converts a Galileosky [`TagSet`] into a [`NormalizedRecord`].
///
/// Returns `None` when the tag set lacks a timestamp, GPS coordinates, or the
/// reported coordinate correctness is not GPS-valid (`0`) or cell-valid (`2`).
///
/// `imei` must be provided by the caller (it is learned per-connection from
/// the first packet's tag 0x03 and carried forward).
///
/// `received_at` is a Unix millisecond timestamp captured by the server at
/// packet-read time, used to track processing latency.
pub fn normalize(imei: u64, tags: &TagSet, received_at: u64) -> Option<NormalizedRecord> {
    let timestamp = tags.timestamp?;
    let coords = tags.coordinates.as_ref()?;

    // Accept GPS-valid (0) and cell-valid (2) coordinates; reject everything else.
    if coords.correctness != 0 && coords.correctness != 2 {
        return None;
    }

    let (speed, angle) = tags
        .speed_direction
        .as_ref()
        .map(|sd| (sd.speed_kmh, sd.direction_deg))
        .unwrap_or((0, 0.0));

    Some(NormalizedRecord {
        imei,
        received_at,
        timestamp,
        longitude: coords.longitude,
        latitude: coords.latitude,
        altitude: tags.altitude.map(|a| a as f32).unwrap_or(0.0),
        angle,
        satellites: coords.satellites,
        speed,
        hdop: tags.hdop.map(|h| h as f32 / 10.0).unwrap_or(0.0),
        can_data: serde_json::json!({}),
    })
}
