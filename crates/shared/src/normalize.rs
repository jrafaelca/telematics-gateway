//! Canonical output structure for a GPS record.
//!
//! [`NormalizedRecord`] is the single source of truth for what fields are
//! published downstream. Both the field set and the serialisation format
//! (decimal precision, units) are defined here via [`NormalizedRecord::to_fields`],
//! so adding or renaming a field only requires a change in this file.

/// A GPS record in the canonical form used for downstream publishing.
///
/// All raw protocol values are already scaled and typed for direct use:
/// coordinates in decimal degrees, altitude in metres, speed in km/h, etc.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NormalizedRecord {
    /// 15-digit device IMEI.
    pub imei: u64,
    /// Wall-clock time at which the server received the packet (Unix ms).
    pub received_at: u64,
    /// Device-reported timestamp (Unix seconds).
    pub timestamp: u32,
    /// Decimal degrees, WGS-84 (positive = East).
    pub longitude: f64,
    /// Decimal degrees, WGS-84 (positive = North).
    pub latitude: f64,
    /// Metres above sea level.
    pub altitude: f32,
    /// Heading in degrees (0–360).
    pub angle: f32,
    pub satellites: u8,
    /// Speed in km/h.
    pub speed: u16,
    /// Horizontal dilution of precision.
    pub hdop: f32,
    /// CAN-bus / IO telemetry serialised as a JSON object.
    pub can_data: serde_json::Value,
}

impl NormalizedRecord {
    /// Returns the record as an ordered list of `(field_name, value)` pairs
    /// ready to pass to a Valkey/Redis `XADD` call.
    ///
    /// This is the **only** place that defines field names and value formatting.
    /// To add, remove, or rename a published field, edit this method.
    pub fn to_fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("imei",        self.imei.to_string()),
            ("received_at", self.received_at.to_string()),
            ("timestamp",   self.timestamp.to_string()),
            ("longitude",   format!("{:.6}", self.longitude)),
            ("latitude",    format!("{:.6}", self.latitude)),
            ("altitude",    format!("{:.1}", self.altitude)),
            ("angle",       format!("{:.2}", self.angle)),
            ("satellites",  self.satellites.to_string()),
            ("speed",       self.speed.to_string()),
            ("hdop",        format!("{:.1}", self.hdop)),
            ("can_data",    self.can_data.to_string()),
        ]
    }
}

/// Returns the Valkey stream key for the given IMEI.
///
/// Records are distributed across `num_shards` streams using
/// `IMEI % num_shards`, producing keys of the form `devices:records:{shard}`.
pub fn stream_key(imei: u64, num_shards: u64) -> String {
    format!("devices:records:{}", imei % num_shards)
}
