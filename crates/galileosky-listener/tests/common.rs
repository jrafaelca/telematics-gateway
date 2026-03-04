use galileosky_listener::crc::crc16_modbus;
use galileosky_listener::protocol::{Packet, parse_packet};

pub fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Head packet from the Galileosky protocol specification (page 5).
///
/// Breakdown:
/// - `01`                     header
/// - `20 00`                  length LE = 32 bytes (no archive)
/// - `01 9A`                  tag 0x01: hardware version = 154
/// - `02 18`                  tag 0x02: firmware version = 24
/// - `03 38..36`              tag 0x03: IMEI = "861230043907626"
/// - `04 32 00`               tag 0x04: device ID = 50
/// - `FE 06 00 01 00 00 00 00 00`  tag 0xFE: extended tags (6 bytes)
/// - `8F 29`                  CRC-16 MODBUS LE = 0x298F
pub const HEAD_PACKET_HEX: &str = concat!(
    "01", "2000",
    "019A",
    "0218",
    "03", "383631323330303433393037363236",
    "04", "3200",
    "FE", "0600", "010000000000",
    "8F29"
);

/// Constructs a simple GPS main packet programmatically, with a correct CRC.
///
/// - IMEI = 861230043907626 (same as head packet)
/// - timestamp = 1 000 000 s
/// - coordinates: lat = 10.0°, lon = 20.0°, satellites = 8, correctness = 0 (GPS valid)
/// - speed = 100 km/h (raw 1000), direction = 90.0° (raw 900)
/// - altitude = 100 m
/// - hdop = 1.0 (raw 10)
pub fn build_main_packet() -> Vec<u8> {
    let mut tags = Vec::new();

    // 0x03: IMEI
    tags.push(0x03u8);
    tags.extend_from_slice(b"861230043907626");

    // 0x20: timestamp = 1_000_000
    tags.push(0x20u8);
    tags.extend_from_slice(&1_000_000u32.to_le_bytes());

    // 0x30: coordinates — 9 bytes
    // byte[0] = (correctness=0)<<4 | satellites=8 = 0x08
    // bytes[1-4] = lat i32 LE (10_000_000)
    // bytes[5-8] = lon i32 LE (20_000_000)
    tags.push(0x30u8);
    tags.push(0x08u8); // correctness=0, satellites=8
    tags.extend_from_slice(&10_000_000i32.to_le_bytes()); // lat = 10.0°
    tags.extend_from_slice(&20_000_000i32.to_le_bytes()); // lon = 20.0°

    // 0x33: speed/direction — 4 bytes
    tags.push(0x33u8);
    tags.extend_from_slice(&1000u16.to_le_bytes()); // speed raw (100 km/h)
    tags.extend_from_slice(&900u16.to_le_bytes());  // dir raw (90.0°)

    // 0x34: altitude — 2 bytes
    tags.push(0x34u8);
    tags.extend_from_slice(&100i16.to_le_bytes());

    // 0x35: hdop — 1 byte (raw 10 → 1.0)
    tags.extend_from_slice(&[0x35, 10]);

    let tag_len = tags.len() as u16;
    let mut frame = vec![0x01u8, (tag_len & 0xFF) as u8, (tag_len >> 8) as u8];
    frame.extend_from_slice(&tags);

    let crc = crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);

    frame
}

/// Parses the head packet from `HEAD_PACKET_HEX` (strips the 2-byte CRC trailer).
#[allow(dead_code)]
pub fn parse_head_packet() -> Packet {
    let data = hex(HEAD_PACKET_HEX);
    parse_packet(&data[..data.len() - 2]).expect("parse_packet head failed")
}

/// Parses the main packet built by `build_main_packet` (strips the 2-byte CRC trailer).
#[allow(dead_code)]
pub fn parse_main_packet() -> Packet {
    let data = build_main_packet();
    parse_packet(&data[..data.len() - 2]).expect("parse_packet main failed")
}
