//! Binary parser for the Ruptela GPS protocol.
//!
//! # Packet layout
//!
//! ```text
//! ┌─ 2 B ──┬─ packet_len B ──────────────────────────────┬─ 2 B ─┐
//! │  len   │  body: IMEI(8) + command_id(1) + payload(…) │ CRC16 │
//! └────────┴─────────────────────────────────────────────┴───────┘
//! ```
//!
//! This module receives the **body slice** (`data[2..2+packet_len]`) after the
//! caller has already stripped the length header and verified the CRC.
//!
//! # Supported commands
//!
//! | `command_id` | Variant                        |
//! |:------------:|--------------------------------|
//! | `0x01`       | [`Payload::Records`]           |
//! | `0x44`       | [`Payload::ExtendedRecords`]   |
//! | any other    | [`Payload::Unknown`]           |

// ── Structures ────────────────────────────────────────────────────────────────

/// A fully-parsed Ruptela packet.
#[derive(Debug)]
pub struct Packet {
    /// Device identifier (15-digit IMEI stored as a 64-bit integer).
    pub imei: u64,
    /// Protocol command identifier (`0x01`, `0x44`, …).
    pub command_id: u8,
    /// Decoded payload, variant-selected by `command_id`.
    pub payload: Payload,
}

/// Decoded payload variants.
#[derive(Debug)]
pub enum Payload {
    /// Command `0x01` — standard GPS records with 1-byte IO IDs.
    Records {
        /// Number of records still queued on the device.
        records_left: u8,
        /// Number of records in this packet.
        num_records: u8,
        records: Vec<Record>,
    },
    /// Command `0x44` — extended GPS records with 2-byte IO IDs and an
    /// extra `record_extension` byte in each record header.
    ExtendedRecords {
        records_left: u8,
        num_records: u8,
        records: Vec<Record>,
    },
    /// Any unrecognised command; the raw payload bytes are preserved.
    Unknown {
        command_id: u8,
        raw: Vec<u8>,
    },
}

/// A single GPS+IO record.
///
/// Field scaling applied during parsing:
/// - `longitude` / `latitude`: raw `i32` ÷ 10 000 000 → decimal degrees (`f64`)
/// - `altitude`: raw `i16` ÷ 10 → metres (`f32`)
/// - `angle`: raw `u16` ÷ 100 → degrees (`f32`)
/// - `hdop`: raw `u8` ÷ 10 → dimensionless (`f32`)
#[derive(Debug)]
pub struct Record {
    /// Unix timestamp (seconds) from the device clock.
    pub timestamp: u32,
    #[allow(dead_code)]
    pub timestamp_ext: u8,
    /// Present only in `0x44` extended records.
    #[allow(dead_code)]
    pub record_extension: Option<u8>,
    #[allow(dead_code)]
    pub priority: u8,
    /// Decimal degrees, WGS-84.
    pub longitude: f64,
    /// Decimal degrees, WGS-84.
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
    /// Trigger event identifier (`u8` for `0x01`, `u16` for `0x44`).
    pub event_id: u16,
    /// All IO elements from the four size-groups (1-, 2-, 4-, 8-byte values).
    pub io: Vec<IoElement>,
}

/// A single IO element: an identifier and its raw integer value.
#[derive(Debug)]
#[allow(dead_code)] // fields will be used when IO normalization is implemented
pub struct IoElement {
    /// IO identifier (1 byte for `0x01` packets, 2 bytes for `0x44`).
    pub id: u16,
    /// Raw value, zero-extended to 64 bits regardless of original size.
    pub value: u64,
}

// ── Public parser API ──────────────────────────────────────────────────────────

/// Parses a Ruptela packet body (without the 2-byte length header or CRC).
///
/// Returns `Err` if the slice is shorter than the minimum 9-byte header
/// (`IMEI` + `command_id`) or if any record inside the payload is malformed.
pub fn parse_packet(data: &[u8]) -> Result<Packet, String> {
    if data.len() < 9 {
        return Err(format!("Packet too short: {} bytes", data.len()));
    }

    let imei = u64::from_be_bytes(data[0..8].try_into().unwrap());
    let command_id = data[8];
    let payload_data = &data[9..];

    let payload = match command_id {
        0x01 => parse_records_payload(payload_data)?,
        0x44 => parse_extended_records_payload(payload_data)?,
        _ => Payload::Unknown {
            command_id,
            raw: payload_data.to_vec(),
        },
    };

    Ok(Packet { imei, command_id, payload })
}

/// Parses a `0x01` Records payload (1-byte IO IDs, 23-byte record headers).
pub fn parse_records_payload(data: &[u8]) -> Result<Payload, String> {
    let (records_left, num_records, records) = parse_records_inner(data, 1, false)?;
    Ok(Payload::Records { records_left, num_records, records })
}

/// Parses a `0x44` ExtendedRecords payload (2-byte IO IDs, 25-byte record headers).
pub fn parse_extended_records_payload(data: &[u8]) -> Result<Payload, String> {
    let (records_left, num_records, records) = parse_records_inner(data, 2, true)?;
    Ok(Payload::ExtendedRecords { records_left, num_records, records })
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Shared records parser.
///
/// - `io_id_size`: bytes per IO identifier (1 for `0x01`, 2 for `0x44`).
/// - `extended_header`: `true` for `0x44` (adds `record_extension` byte and
///   widens `event_id` to 2 bytes, making the header 25 bytes instead of 23).
fn parse_records_inner(data: &[u8], io_id_size: usize, extended_header: bool) -> Result<(u8, u8, Vec<Record>), String> {
    if data.len() < 2 {
        return Err("Records payload too short".to_string());
    }

    let records_left = data[0];
    let num_records = data[1];
    let mut records = Vec::new();
    let mut cursor = 2;

    for i in 0..num_records {
        match parse_record(&data[cursor..], io_id_size, extended_header) {
            Ok((record, consumed)) => {
                records.push(record);
                cursor += consumed;
            }
            Err(e) => return Err(format!("Error in record #{}: {}", i + 1, e)),
        }
    }

    Ok((records_left, num_records, records))
}

/// Parses a single record from `data`, returning the record and the number of
/// bytes consumed so the caller can advance its cursor.
fn parse_record(data: &[u8], io_id_size: usize, extended_header: bool) -> Result<(Record, usize), String> {
    // 0x01: 23-byte header (event_id 1 B)
    // 0x44: 25-byte header (record_extension 1 B + event_id 2 B)
    let header_size = if extended_header { 25 } else { 23 };
    if data.len() < header_size {
        return Err(format!("Record header too short: {} bytes", data.len()));
    }

    let timestamp     = u32::from_be_bytes(data[0..4].try_into().unwrap());
    let timestamp_ext = data[4];

    // Offset of the longitude field depends on whether record_extension is present.
    let (record_extension, priority, lon) = if extended_header {
        (Some(data[5]), data[6], 7usize)
    } else {
        (None, data[5], 6usize)
    };

    let longitude  = i32::from_be_bytes(data[lon..lon+4].try_into().unwrap()) as f64 / 10_000_000.0;
    let latitude   = i32::from_be_bytes(data[lon+4..lon+8].try_into().unwrap()) as f64 / 10_000_000.0;
    let altitude   = i16::from_be_bytes(data[lon+8..lon+10].try_into().unwrap()) as f32 / 10.0;
    let angle      = u16::from_be_bytes(data[lon+10..lon+12].try_into().unwrap()) as f32 / 100.0;
    let satellites = data[lon+12];
    let speed      = u16::from_be_bytes(data[lon+13..lon+15].try_into().unwrap());
    let hdop       = data[lon+15] as f32 / 10.0;
    let event_id   = if extended_header {
        u16::from_be_bytes(data[lon+16..lon+18].try_into().unwrap())
    } else {
        data[lon+16] as u16
    };

    let mut cursor = header_size;
    let mut io = Vec::new();

    // Parse one IO group: `count` × (id + value) pairs of the given value size.
    let read_io_group = |data: &[u8], cursor: &mut usize, val_size: usize| -> Result<Vec<IoElement>, String> {
        if *cursor >= data.len() {
            return Err(format!("Missing IO {}B data", val_size));
        }
        let count = data[*cursor] as usize;
        *cursor += 1;
        let mut elements = Vec::with_capacity(count);
        for _ in 0..count {
            if *cursor + io_id_size + val_size > data.len() {
                return Err(format!("IO {}B data truncated", val_size));
            }
            let id = match io_id_size {
                1 => data[*cursor] as u16,
                2 => u16::from_be_bytes(data[*cursor..*cursor + 2].try_into().unwrap()),
                _ => unreachable!(),
            };
            let value = match val_size {
                1 => data[*cursor + io_id_size] as u64,
                2 => u16::from_be_bytes(data[*cursor + io_id_size..*cursor + io_id_size + 2].try_into().unwrap()) as u64,
                4 => u32::from_be_bytes(data[*cursor + io_id_size..*cursor + io_id_size + 4].try_into().unwrap()) as u64,
                8 => u64::from_be_bytes(data[*cursor + io_id_size..*cursor + io_id_size + 8].try_into().unwrap()),
                _ => unreachable!(),
            };
            elements.push(IoElement { id, value });
            *cursor += io_id_size + val_size;
        }
        Ok(elements)
    };

    // Four successive groups: 1-byte, 2-byte, 4-byte, 8-byte values.
    io.extend(read_io_group(data, &mut cursor, 1)?);
    io.extend(read_io_group(data, &mut cursor, 2)?);
    io.extend(read_io_group(data, &mut cursor, 4)?);
    io.extend(read_io_group(data, &mut cursor, 8)?);

    Ok((
        Record { timestamp, timestamp_ext, record_extension, priority, longitude, latitude, altitude, angle, satellites, speed, hdop, event_id, io },
        cursor,
    ))
}
