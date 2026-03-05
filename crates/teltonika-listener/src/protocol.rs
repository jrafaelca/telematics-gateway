//! Binary parser for the Teltonika AVL protocol (Codec 8 and Codec 8 Extended).
//!
//! # Packet layout (TCP)
//!
//! ```text
//! ┌─ 4 B ──────┬─ 4 B BE ──────────┬─ dfl B ───────────────────────────┬─ 4 B BE ─┐
//! │  preamble  │ data_field_length  │  codec_id + records + num_data_2  │  CRC-16  │
//! └────────────┴───────────────────┴───────────────────────────────────┴──────────┘
//! ```
//!
//! This module receives the **data field slice** (`data[0..dfl]`) after the
//! caller has already validated the CRC-16/IBM checksum.
//!
//! # Data field layout
//!
//! ```text
//! [1 B: codec_id]  [1 B: num_data_1]  [AVL records × num_data_1]  [1 B: num_data_2]
//! ```
//!
//! `num_data_1` must equal `num_data_2`; the server ACKs with `num_data_1` as
//! a 4-byte big-endian integer.
//!
//! # Codec IDs
//!
//! - `0x08` — Codec 8: 1-byte IO IDs and counts.
//! - `0x8E` — Codec 8 Extended: 2-byte IO IDs and counts, plus variable-length IOs.

// ── Structures ─────────────────────────────────────────────────────────────────

/// A fully-parsed AVL packet (data field only; preamble and CRC excluded).
#[derive(Debug)]
pub struct AvlPacket {
    /// Codec identifier: `0x08` = Codec 8, `0x8E` = Codec 8 Extended.
    pub codec_id: u8,
    /// Parsed AVL records.
    pub records: Vec<AvlRecord>,
}

/// A single AVL data record.
#[derive(Debug, Clone)]
pub struct AvlRecord {
    /// Device-reported time as Unix milliseconds.
    pub timestamp_ms: u64,
    /// Priority: `0` = Low, `1` = High, `2` = Panic.
    pub priority: u8,
    /// Longitude in decimal degrees (WGS-84). Positive = East.
    pub longitude: f64,
    /// Latitude in decimal degrees (WGS-84). Positive = North.
    pub latitude: f64,
    /// Altitude in metres above sea level.
    pub altitude: i16,
    /// Heading in degrees from north (0–359).
    pub angle: u16,
    /// Number of satellites used for the fix.
    pub satellites: u8,
    /// Speed in km/h.
    pub speed: u16,
    /// IO element that triggered this record (`0` = not IO-driven).
    pub event_io_id: u16,
    /// All IO elements: `(id, value)` pairs.
    pub io_elements: Vec<IoElement>,
}

/// A single IO element from an AVL record.
#[derive(Debug, Clone)]
pub struct IoElement {
    pub id: u16,
    pub value: IoValue,
}

/// Value variants for IO elements.
#[derive(Debug, Clone)]
pub enum IoValue {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    /// Variable-length (Codec 8 Extended NX section).
    Bytes(Vec<u8>),
}

// ── Public parser API ──────────────────────────────────────────────────────────

/// Parses an AVL data field (without preamble and CRC trailer).
///
/// `data` must be the bytes from `codec_id` through `num_data_2` inclusive.
///
/// Returns `Err` if the slice is too short, the codec ID is unsupported, or
/// any record field is truncated.
pub fn parse_packet(data: &[u8]) -> Result<AvlPacket, String> {
    if data.len() < 3 {
        return Err(format!("Data field too short: {} bytes (minimum 3)", data.len()));
    }

    let codec_id = data[0];
    if codec_id != 0x08 && codec_id != 0x8E {
        return Err(format!("Unsupported codec ID: 0x{codec_id:02X}"));
    }

    let num_data_1 = data[1] as usize;
    let mut cursor = 2usize;
    let mut records = Vec::with_capacity(num_data_1);

    for i in 0..num_data_1 {
        let (record, consumed) = parse_record(&data[cursor..], codec_id)
            .map_err(|e| format!("Record {i}: {e}"))?;
        records.push(record);
        cursor += consumed;
    }

    if cursor >= data.len() {
        return Err("Missing num_data_2 byte".to_string());
    }
    let num_data_2 = data[cursor] as usize;
    if num_data_2 != num_data_1 {
        return Err(format!(
            "num_data_1 ({num_data_1}) != num_data_2 ({num_data_2})"
        ));
    }

    Ok(AvlPacket { codec_id, records })
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Parses a single AVL record from `data`, returning the record and the number
/// of bytes consumed.
fn parse_record(data: &[u8], codec_id: u8) -> Result<(AvlRecord, usize), String> {
    let mut c = 0usize;

    // Timestamp — 8 bytes, u64 BE, milliseconds.
    let timestamp_ms = read_u64_be(data, &mut c, "timestamp")?;

    // Priority — 1 byte.
    let priority = read_u8(data, &mut c, "priority")?;

    // GPS element — 15 bytes.
    let lon_raw = read_i32_be(data, &mut c, "longitude")?;
    let lat_raw = read_i32_be(data, &mut c, "latitude")?;
    let altitude = read_i16_be(data, &mut c, "altitude")?;
    let angle = read_u16_be(data, &mut c, "angle")?;
    let satellites = read_u8(data, &mut c, "satellites")?;
    let speed = read_u16_be(data, &mut c, "speed")?;

    // IO element.
    let (event_io_id, io_elements) = parse_io_element(data, &mut c, codec_id)?;

    let record = AvlRecord {
        timestamp_ms,
        priority,
        longitude: lon_raw as f64 / 10_000_000.0,
        latitude: lat_raw as f64 / 10_000_000.0,
        altitude,
        angle,
        satellites,
        speed,
        event_io_id,
        io_elements,
    };

    Ok((record, c))
}

/// Parses the IO element section, returning `(event_io_id, elements)`.
fn parse_io_element(
    data: &[u8],
    c: &mut usize,
    codec_id: u8,
) -> Result<(u16, Vec<IoElement>), String> {
    let extended = codec_id == 0x8E;

    let event_io_id = if extended {
        read_u16_be(data, c, "event_io_id")?
    } else {
        read_u8(data, c, "event_io_id")? as u16
    };

    let _n_total = if extended {
        read_u16_be(data, c, "n_total")?
    } else {
        read_u8(data, c, "n_total")? as u16
    };

    let mut elements: Vec<IoElement> = Vec::new();

    // N1 — 1-byte values.
    let n1 = if extended {
        read_u16_be(data, c, "n1")? as usize
    } else {
        read_u8(data, c, "n1")? as usize
    };
    for _ in 0..n1 {
        let id = read_io_id(data, c, extended)?;
        let v = read_u8(data, c, "IO u8 value")?;
        elements.push(IoElement { id, value: IoValue::U8(v) });
    }

    // N2 — 2-byte values.
    let n2 = if extended {
        read_u16_be(data, c, "n2")? as usize
    } else {
        read_u8(data, c, "n2")? as usize
    };
    for _ in 0..n2 {
        let id = read_io_id(data, c, extended)?;
        let v = read_u16_be(data, c, "IO u16 value")?;
        elements.push(IoElement { id, value: IoValue::U16(v) });
    }

    // N4 — 4-byte values.
    let n4 = if extended {
        read_u16_be(data, c, "n4")? as usize
    } else {
        read_u8(data, c, "n4")? as usize
    };
    for _ in 0..n4 {
        let id = read_io_id(data, c, extended)?;
        let v = read_u32_be(data, c, "IO u32 value")?;
        elements.push(IoElement { id, value: IoValue::U32(v) });
    }

    // N8 — 8-byte values.
    let n8 = if extended {
        read_u16_be(data, c, "n8")? as usize
    } else {
        read_u8(data, c, "n8")? as usize
    };
    for _ in 0..n8 {
        let id = read_io_id(data, c, extended)?;
        let v = read_u64_be(data, c, "IO u64 value")?;
        elements.push(IoElement { id, value: IoValue::U64(v) });
    }

    // NX — variable-length values (Codec 8 Extended only).
    if extended {
        let nx = read_u16_be(data, c, "nx")? as usize;
        for _ in 0..nx {
            let id = read_u16_be(data, c, "IO NX id")?;
            let len = read_u16_be(data, c, "IO NX len")? as usize;
            if *c + len > data.len() {
                return Err(format!(
                    "IO NX id={id}: data truncated (need {len}, have {})",
                    data.len().saturating_sub(*c),
                ));
            }
            let bytes = data[*c..*c + len].to_vec();
            *c += len;
            elements.push(IoElement { id, value: IoValue::Bytes(bytes) });
        }
    }

    Ok((event_io_id, elements))
}

// ── Primitive readers ──────────────────────────────────────────────────────────

fn read_u8(data: &[u8], c: &mut usize, field: &str) -> Result<u8, String> {
    if *c >= data.len() {
        return Err(format!("{field}: unexpected end of data"));
    }
    let v = data[*c];
    *c += 1;
    Ok(v)
}

fn read_u16_be(data: &[u8], c: &mut usize, field: &str) -> Result<u16, String> {
    if *c + 2 > data.len() {
        return Err(format!("{field}: unexpected end of data"));
    }
    let v = u16::from_be_bytes([data[*c], data[*c + 1]]);
    *c += 2;
    Ok(v)
}

fn read_i16_be(data: &[u8], c: &mut usize, field: &str) -> Result<i16, String> {
    if *c + 2 > data.len() {
        return Err(format!("{field}: unexpected end of data"));
    }
    let v = i16::from_be_bytes([data[*c], data[*c + 1]]);
    *c += 2;
    Ok(v)
}

fn read_i32_be(data: &[u8], c: &mut usize, field: &str) -> Result<i32, String> {
    if *c + 4 > data.len() {
        return Err(format!("{field}: unexpected end of data"));
    }
    let v = i32::from_be_bytes([data[*c], data[*c + 1], data[*c + 2], data[*c + 3]]);
    *c += 4;
    Ok(v)
}

fn read_u32_be(data: &[u8], c: &mut usize, field: &str) -> Result<u32, String> {
    if *c + 4 > data.len() {
        return Err(format!("{field}: unexpected end of data"));
    }
    let v = u32::from_be_bytes([data[*c], data[*c + 1], data[*c + 2], data[*c + 3]]);
    *c += 4;
    Ok(v)
}

fn read_u64_be(data: &[u8], c: &mut usize, field: &str) -> Result<u64, String> {
    if *c + 8 > data.len() {
        return Err(format!("{field}: unexpected end of data"));
    }
    let v = u64::from_be_bytes([
        data[*c],
        data[*c + 1],
        data[*c + 2],
        data[*c + 3],
        data[*c + 4],
        data[*c + 5],
        data[*c + 6],
        data[*c + 7],
    ]);
    *c += 8;
    Ok(v)
}

/// Reads a 1-byte or 2-byte IO element ID depending on the codec.
fn read_io_id(data: &[u8], c: &mut usize, extended: bool) -> Result<u16, String> {
    if extended {
        read_u16_be(data, c, "IO id")
    } else {
        Ok(read_u8(data, c, "IO id")? as u16)
    }
}
