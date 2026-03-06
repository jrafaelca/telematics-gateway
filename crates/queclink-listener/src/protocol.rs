//! Queclink @Track ASCII protocol parser.
//!
//! Each Queclink message is a single text line terminated with `\r\n`.
//! The line content ends with a `$` followed by the line terminator.
//! There is no CRC — the 4-hex-digit tail before `$` is a count number.
//!
//! # Supported messages
//!
//! | Prefix            | Variant           |
//! |-------------------|-------------------|
//! | `+RESP:GTFRI`     | `FriReport`       |
//! | `+BUFF:GTFRI`     | `FriReport`       |
//! | `+RESP:GT{LOC}`   | `LocationReport`  |
//! | `+BUFF:GT{LOC}`   | `LocationReport`  |
//! | `+ACK:GTHBD`      | `Heartbeat`       |
//! | `+ACK:GT{CMD}`    | `CommandAck`      |

/// Parsed Queclink message.
#[derive(Debug)]
pub enum Message {
    /// `+RESP:GTFRI` or `+BUFF:GTFRI` — primary GPS report.
    FriReport(FriRecord),
    /// Generic location reports sharing the GTFRI field layout.
    LocationReport(FriRecord),
    /// `+ACK:GTHBD` — device heartbeat.
    Heartbeat(HbdRecord),
    /// `+ACK:GT{CMD}` — device confirms a server-initiated command.
    CommandAck(AckRecord),
    /// Unrecognised or unparseable line.
    Unknown,
}

/// GPS report record (GTFRI and generic location messages).
#[derive(Debug, Clone)]
pub struct FriRecord {
    /// Message type without the `GT` prefix (e.g. `"FRI"`, `"TOW"`).
    pub msg_type: String,
    /// Protocol version (6 hex chars from the device).
    pub version: String,
    /// Device IMEI (15 digits).
    pub imei: u64,
    /// Device name as configured on the device.
    pub device_name: String,
    /// GNSS accuracy / HDOP proxy reported by the device (0 = no fix).
    pub gnss_accuracy: f64,
    /// Speed in km/h.
    pub speed: f64,
    /// Azimuth / heading in degrees.
    pub azimuth: f64,
    /// Altitude in metres.
    pub altitude: f64,
    /// Longitude in decimal degrees (WGS-84, positive = East).
    pub longitude: f64,
    /// Latitude in decimal degrees (WGS-84, positive = North).
    pub latitude: f64,
    /// GNSS UTC timestamp converted to Unix seconds.
    pub timestamp: u32,
    /// Number of GNSS satellites used (0 if not reported).
    pub satellites: u8,
    /// HDOP value (from optional field or `gnss_accuracy` when absent).
    pub hdop: f64,
    /// 4-hex-digit count number from the tail of the message.
    pub count: String,
}

/// Heartbeat record (`+ACK:GTHBD`).
#[derive(Debug, Clone)]
pub struct HbdRecord {
    pub version: String,
    pub imei: u64,
    pub device_name: String,
    /// 4-hex-digit count number.
    pub count: String,
}

/// Command ACK record (`+ACK:GT{CMD}`).
#[derive(Debug, Clone)]
pub struct AckRecord {
    /// Command name without the `GT` prefix (e.g. `"RTO"`).
    pub msg_type: String,
    pub version: String,
    pub imei: u64,
    pub device_name: String,
    /// Serial number echoed from the server command (hex string, 4 chars).
    pub serial_num: String,
    pub count: String,
}

/// Message types that share the GTFRI GPS field layout.
const GPS_MSG_TYPES: &[&str] = &[
    "FRI", "TOW", "DIS", "IOB", "SPD", "SOS", "RTL", "DOG", "IGL", "VGL", "HBM", "SPA",
];

/// Parses a single Queclink ASCII line into a [`Message`].
///
/// The `line` may contain a trailing `\r\n`.  The trailing `$` (before the
/// line terminator) and the `\r\n` are stripped internally before parsing.
pub fn parse_line(line: &str) -> Message {
    // Strip \r\n and trailing $
    let line = line.trim_end_matches(['\r', '\n']).trim_end_matches('$');
    if line.is_empty() {
        return Message::Unknown;
    }

    let fields: Vec<&str> = line.split(',').collect();
    let tag = match fields.first() {
        Some(t) => *t,
        None => return Message::Unknown,
    };

    if tag == "+ACK:GTHBD" {
        return parse_hbd(&fields);
    }

    // Extract prefix (+RESP, +BUFF, +ACK) and message type (GTXXX)
    let (prefix, gt_type) = match tag.split_once(':') {
        Some(parts) => parts,
        None => return Message::Unknown,
    };

    let msg_type = gt_type.strip_prefix("GT").unwrap_or("");
    if msg_type.is_empty() {
        return Message::Unknown;
    }

    match prefix {
        "+RESP" | "+BUFF" => {
            if msg_type == "FRI" {
                match parse_fri_fields(&fields, msg_type) {
                    Some(rec) => Message::FriReport(rec),
                    None => Message::Unknown,
                }
            } else if GPS_MSG_TYPES.contains(&msg_type) {
                match parse_fri_fields(&fields, msg_type) {
                    Some(rec) => Message::LocationReport(rec),
                    None => Message::Unknown,
                }
            } else {
                Message::Unknown
            }
        }
        "+ACK" => {
            // +ACK:GTHBD was handled above; all others are command ACKs.
            parse_cmd_ack(&fields, msg_type)
        }
        _ => Message::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Field parsers
// ---------------------------------------------------------------------------

fn parse_fri_fields(fields: &[&str], msg_type: &str) -> Option<FriRecord> {
    // Minimum layout: indices 0..18 (19 fields) + send_time + count = 21 total.
    if fields.len() < 21 {
        return None;
    }

    let version = fields[1].to_string();
    let imei: u64 = fields[2].parse().ok()?;
    let device_name = fields[3].to_string();
    let gnss_accuracy: f64 = fields[7].parse().ok()?;
    let speed: f64 = fields[8].parse().unwrap_or(0.0);
    let azimuth: f64 = fields[9].parse().unwrap_or(0.0);
    let altitude: f64 = fields[10].parse().unwrap_or(0.0);
    let longitude: f64 = fields[11].parse().ok()?;
    let latitude: f64 = fields[12].parse().ok()?;
    let timestamp = parse_datetime(fields[13])?;

    // Position Append Mask at index 18 (2 hex chars).
    let mask = u8::from_str_radix(fields[18], 16).unwrap_or(0);
    let mut optional_idx = 19usize;

    let satellites: u8 = if mask & 0x01 != 0 {
        let s = fields
            .get(optional_idx)
            .and_then(|f| f.parse().ok())
            .unwrap_or(0);
        optional_idx += 1;
        s
    } else {
        // Default: 1 satellite if a fix is reported, 0 otherwise.
        if gnss_accuracy > 0.0 { 1 } else { 0 }
    };

    let hdop: f64 = if mask & 0x02 != 0 {
        fields
            .get(optional_idx)
            .and_then(|f| f.parse().ok())
            .unwrap_or(gnss_accuracy)
    } else {
        gnss_accuracy
    };

    // count is always the last field, send_time is second-to-last.
    let n = fields.len();
    let count = fields[n - 1].to_string();

    Some(FriRecord {
        msg_type: msg_type.to_string(),
        version,
        imei,
        device_name,
        gnss_accuracy,
        speed,
        azimuth,
        altitude,
        longitude,
        latitude,
        timestamp,
        satellites,
        hdop,
        count,
    })
}

fn parse_hbd(fields: &[&str]) -> Message {
    // +ACK:GTHBD,{version},{IMEI},{device_name},{send_time},{count}$
    if fields.len() < 6 {
        return Message::Unknown;
    }
    let version = fields[1].to_string();
    let imei: u64 = match fields[2].parse() {
        Ok(v) => v,
        Err(_) => return Message::Unknown,
    };
    let device_name = fields[3].to_string();
    let n = fields.len();
    let count = fields[n - 1].to_string();
    Message::Heartbeat(HbdRecord { version, imei, device_name, count })
}

fn parse_cmd_ack(fields: &[&str], msg_type: &str) -> Message {
    // +ACK:GT{CMD},{version},{IMEI},{device_name},{subcmd},{serial_num},{send_time},{count}$
    if fields.len() < 8 {
        return Message::Unknown;
    }
    let version = fields[1].to_string();
    let imei: u64 = match fields[2].parse() {
        Ok(v) => v,
        Err(_) => return Message::Unknown,
    };
    let device_name = fields[3].to_string();
    let serial_num = fields[5].to_string();
    let n = fields.len();
    let count = fields[n - 1].to_string();
    Message::CommandAck(AckRecord {
        msg_type: msg_type.to_string(),
        version,
        imei,
        device_name,
        serial_num,
        count,
    })
}

// ---------------------------------------------------------------------------
// Date/time helper
// ---------------------------------------------------------------------------

/// Converts a `YYYYMMDDHHMMSS` string to a Unix timestamp (seconds since epoch).
///
/// Returns `None` if the string is not exactly 14 characters or contains
/// non-numeric characters.
pub fn parse_datetime(s: &str) -> Option<u32> {
    if s.len() != 14 {
        return None;
    }
    let year: u32 = s[0..4].parse().ok()?;
    let month: u32 = s[4..6].parse().ok()?;
    let day: u32 = s[6..8].parse().ok()?;
    let hour: u32 = s[8..10].parse().ok()?;
    let min: u32 = s[10..12].parse().ok()?;
    let sec: u32 = s[12..14].parse().ok()?;

    if year < 1970 || month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }

    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    fn is_leap(y: u32) -> bool {
        (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
    }

    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += DAYS_IN_MONTH[(m - 1) as usize] as u64;
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    days += (day - 1) as u64;

    let secs = days * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64;
    Some(secs as u32)
}
