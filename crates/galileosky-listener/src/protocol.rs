//! Binary parser for the Galileosky GPS protocol.
//!
//! # Packet layout
//!
//! ```text
//! ┌─ 1 B ──┬─ 2 B LE ──────────────┬─ tag_len B ──┬─ 2 B LE ─┐
//! │ header │ raw_len (bit15=archive)│   tags (TLV) │ CRC16    │
//! └────────┴───────────────────────┴──────────────┴──────────┘
//! ```
//!
//! This module receives the **body slice** (`data[0..1+2+tag_len]`) after the
//! caller has already stripped the 2-byte CRC trailer and validated the
//! checksum.
//!
//! # Tag encoding
//!
//! Tags are TLV with 1-byte IDs (except extended section 0xFE which has
//! 2-byte IDs internally).  The data length per tag is fixed by the tag ID
//! according to the Galileosky protocol specification.

// ── Structures ─────────────────────────────────────────────────────────────────

/// A fully-parsed Galileosky packet (without the 2-byte CRC).
#[derive(Debug)]
pub struct Packet {
    /// Header byte: `0x01` = standard, `0x08` = compressed.
    pub header: u8,
    /// True when bit 15 of the raw length field is set (device has unsent archive data).
    pub is_archive: bool,
    /// All decoded tags from the tag section.
    pub tags: TagSet,
}

/// Decoded tag values from a single Galileosky packet.
#[derive(Debug, Default)]
pub struct TagSet {
    /// Tag 0x01 — hardware version (1 B).
    pub hardware_version: Option<u8>,
    /// Tag 0x02 — firmware version (1 B).
    pub firmware_version: Option<u8>,
    /// Tag 0x03 — IMEI (15 ASCII bytes, parsed as `u64`).
    pub imei: Option<u64>,
    /// Tag 0x04 — device identifier (2 B LE).
    pub device_id: Option<u16>,
    /// Tag 0x10 — archive record number (2 B LE).
    pub archive_record_no: Option<u16>,
    /// Tag 0x20 — Unix timestamp in seconds (4 B LE).
    pub timestamp: Option<u32>,
    /// Tag 0x30 — GPS coordinates (9 B).
    pub coordinates: Option<Coordinates>,
    /// Tag 0x33 — speed and heading (4 B LE).
    pub speed_direction: Option<SpeedDir>,
    /// Tag 0x34 — altitude in metres (2 B LE, signed).
    pub altitude: Option<i16>,
    /// Tag 0x35 — HDOP × 10 (1 B; divide by 10 for the real value).
    pub hdop: Option<u8>,
    /// Tag 0xE0 — server→device command number (4 B LE).
    pub command_number: Option<u32>,
    /// Tag 0xE1 — command text (1 B length prefix + N bytes, decoded as UTF-8).
    pub command_text: Option<String>,
    /// Tag 0xFE — raw content of the extended-tags section (after the 2-byte length).
    pub extended_raw: Vec<u8>,
    /// Any other known-size tags preserved as raw bytes for debugging.
    pub extra: Vec<(u8, Vec<u8>)>,
}

/// Decoded coordinates from tag 0x30 (9 bytes).
#[derive(Debug, Clone)]
pub struct Coordinates {
    /// Number of satellites used for the fix (bits 3-0 of byte 0).
    pub satellites: u8,
    /// Coordinate validity: `0` = GPS valid, `2` = cell-based valid, others = invalid.
    /// Derived from bits 7-4 of byte 0.
    pub correctness: u8,
    /// Latitude in decimal degrees (WGS-84): bytes 1-4 as `i32 LE` ÷ 1 000 000.
    pub latitude: f64,
    /// Longitude in decimal degrees (WGS-84): bytes 5-8 as `i32 LE` ÷ 1 000 000.
    pub longitude: f64,
}

/// Decoded speed and heading from tag 0x33 (4 bytes).
#[derive(Debug, Clone)]
pub struct SpeedDir {
    /// Speed in km/h: bytes 0-1 as `u16 LE` ÷ 10, rounded.
    pub speed_kmh: u16,
    /// Heading in degrees (0-360): bytes 2-3 as `u16 LE` ÷ 10.
    pub direction_deg: f32,
}

// ── Tag size classification ────────────────────────────────────────────────────

enum TagSize {
    /// Fixed number of data bytes following the tag ID byte.
    Fixed(usize),
    /// Variable: 1-byte length prefix immediately follows the tag ID.
    VarLen1,
    /// Variable: 2-byte LE length prefix immediately follows the tag ID.
    VarLen2,
    /// Tag ID is not in the known table; parsing stops.
    Unknown,
}

/// Returns the data-size class for a tag ID.
///
/// Sizes follow the Galileosky protocol specification.  Tags not listed here
/// are treated as `Unknown`; on encountering an unknown tag the parser logs a
/// warning and stops consuming bytes.
fn tag_data_size(tag: u8) -> TagSize {
    match tag {
        // 1-byte data
        0x01 | 0x02 | 0x35 | 0x43 | 0xC4..=0xD2 => TagSize::Fixed(1),
        // 2-byte data
        0x04 | 0x10 | 0x34 | 0x40 | 0x41 | 0x42 | 0x45 | 0x46
        | 0x50..=0x59 | 0x70..=0x77 | 0xD6..=0xD9 => TagSize::Fixed(2),
        // 4-byte data
        0x20 | 0x33 | 0x44 | 0x90 | 0xC0..=0xC3 | 0xD4
        | 0xDB..=0xDF | 0xE0 | 0xE2..=0xE9 => TagSize::Fixed(4),
        // 9-byte data
        0x30 => TagSize::Fixed(9),
        // 15-byte data (IMEI as ASCII digits)
        0x03 => TagSize::Fixed(15),
        // Variable: 1B length prefix + N bytes
        0xE1 | 0xEA => TagSize::VarLen1,
        // Variable: 2B LE length prefix + N bytes (extended-tags section)
        0xFE => TagSize::VarLen2,
        // Unknown tag ID
        _ => TagSize::Unknown,
    }
}

// ── Public parser API ──────────────────────────────────────────────────────────

/// Parses a Galileosky packet without the 2-byte CRC trailer.
///
/// `data` must be `[header (1 B)][raw_len LE (2 B)][tags (tag_len B)]`.
///
/// Returns `Err` if the slice is shorter than 3 bytes (minimum framing), the
/// declared tag length exceeds the slice, or a required tag field is truncated.
pub fn parse_packet(data: &[u8]) -> Result<Packet, String> {
    if data.len() < 3 {
        return Err(format!("Packet too short: {} bytes (need at least 3)", data.len()));
    }

    let header = data[0];
    let raw_len = u16::from_le_bytes([data[1], data[2]]);
    let is_archive = (raw_len & 0x8000) != 0;
    let tag_len = (raw_len & 0x7FFF) as usize;

    if data.len() < 3 + tag_len {
        return Err(format!(
            "Packet data too short: expected {} tag bytes, have {}",
            tag_len,
            data.len().saturating_sub(3),
        ));
    }

    let tag_data = &data[3..3 + tag_len];
    let tags = parse_tags(tag_data)?;

    Ok(Packet { header, is_archive, tags })
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Sequentially parses all TLV tags from `data`.
///
/// Stops early (without error) when an unknown tag ID is encountered, logging
/// a `WARN` so that the partially-parsed `TagSet` is still usable.
fn parse_tags(data: &[u8]) -> Result<TagSet, String> {
    let mut tags = TagSet::default();
    let mut cursor = 0usize;

    while cursor < data.len() {
        let tag_id = data[cursor];
        cursor += 1;

        let data_size = match tag_data_size(tag_id) {
            TagSize::Fixed(n) => n,
            TagSize::VarLen1 => {
                if cursor >= data.len() {
                    return Err(format!("Tag 0x{tag_id:02X}: missing 1-byte length"));
                }
                let n = data[cursor] as usize;
                cursor += 1;
                n
            }
            TagSize::VarLen2 => {
                if cursor + 1 >= data.len() {
                    return Err(format!("Tag 0x{tag_id:02X}: missing 2-byte length"));
                }
                let n = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
                cursor += 2;
                n
            }
            TagSize::Unknown => {
                tracing::warn!(
                    tag = format_args!("0x{:02X}", tag_id),
                    "unknown tag ID, stopping tag parse"
                );
                break;
            }
        };

        if cursor + data_size > data.len() {
            return Err(format!(
                "Tag 0x{tag_id:02X}: data truncated (need {data_size}, have {})",
                data.len() - cursor,
            ));
        }

        let field = &data[cursor..cursor + data_size];
        cursor += data_size;

        apply_tag(&mut tags, tag_id, field);
    }

    Ok(tags)
}

/// Applies a single decoded tag field to the `TagSet`.
fn apply_tag(tags: &mut TagSet, id: u8, data: &[u8]) {
    match id {
        0x01 => tags.hardware_version = Some(data[0]),
        0x02 => tags.firmware_version = Some(data[0]),
        0x03 => {
            if let Ok(s) = std::str::from_utf8(data) {
                if let Ok(imei) = s.trim().parse::<u64>() {
                    tags.imei = Some(imei);
                }
            }
        }
        0x04 => tags.device_id = Some(u16::from_le_bytes([data[0], data[1]])),
        0x10 => tags.archive_record_no = Some(u16::from_le_bytes([data[0], data[1]])),
        0x20 => {
            tags.timestamp = Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
        }
        0x30 => {
            let byte0 = data[0];
            let satellites = byte0 & 0x0F;
            let correctness = (byte0 >> 4) & 0x0F;
            let lat_raw = i32::from_le_bytes([data[1], data[2], data[3], data[4]]);
            let lon_raw = i32::from_le_bytes([data[5], data[6], data[7], data[8]]);
            tags.coordinates = Some(Coordinates {
                satellites,
                correctness,
                latitude: lat_raw as f64 / 1_000_000.0,
                longitude: lon_raw as f64 / 1_000_000.0,
            });
        }
        0x33 => {
            let raw_speed = u16::from_le_bytes([data[0], data[1]]);
            let raw_dir = u16::from_le_bytes([data[2], data[3]]);
            tags.speed_direction = Some(SpeedDir {
                speed_kmh: (raw_speed as f32 / 10.0).round() as u16,
                direction_deg: raw_dir as f32 / 10.0,
            });
        }
        0x34 => tags.altitude = Some(i16::from_le_bytes([data[0], data[1]])),
        0x35 => tags.hdop = Some(data[0]),
        0xE0 => {
            tags.command_number =
                Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
        }
        0xE1 => {
            // `data` is already the text bytes (length prefix was consumed by parse_tags).
            tags.command_text = Some(String::from_utf8_lossy(data).into_owned());
        }
        0xFE => {
            // Store the entire extended-tags section as raw bytes.
            tags.extended_raw = data.to_vec();
        }
        _ => {
            // Known-size tag without a named field — preserve for debugging.
            tags.extra.push((id, data.to_vec()));
        }
    }
}
