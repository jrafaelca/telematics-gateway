mod common;
use galileosky_listener::protocol::parse_packet;

#[test]
fn test_parse_head_packet_imei() {
    let packet = common::parse_head_packet();
    assert_eq!(packet.tags.imei, Some(861230043907626u64));
}

#[test]
fn test_parse_head_packet_hw_fw() {
    let packet = common::parse_head_packet();
    assert_eq!(packet.tags.hardware_version, Some(0x9A)); // 154
    assert_eq!(packet.tags.firmware_version, Some(0x18)); // 24
}

#[test]
fn test_parse_head_packet_device_id() {
    let packet = common::parse_head_packet();
    assert_eq!(packet.tags.device_id, Some(50));
}

#[test]
fn test_parse_head_packet_archive_flag() {
    // The head packet from the spec has raw_len = 0x0020, so is_archive = false.
    let packet = common::parse_head_packet();
    assert!(!packet.is_archive);
}

#[test]
fn test_parse_archive_flag_set() {
    // Construct a minimal packet with bit 15 of raw_len set.
    // Tags: just tag 0x35 (hdop=0, 1 byte) → tag_len = 2.
    // raw_len = 0x8002 → is_archive = true, tag_len = 2.
    let mut frame = vec![0x01u8, 0x02, 0x80, 0x35, 0x00];
    let crc = galileosky_listener::crc::crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    let without_crc = &frame[..frame.len() - 2];
    let packet = parse_packet(without_crc).expect("parse failed");
    assert!(packet.is_archive);
    assert_eq!(packet.tags.hdop, Some(0));
}

#[test]
fn test_parse_main_packet_gps() {
    let packet = common::parse_main_packet();
    assert_eq!(packet.tags.timestamp, Some(1_000_000));
    let coords = packet.tags.coordinates.as_ref().expect("no coordinates");
    assert_eq!(coords.satellites, 8);
    assert_eq!(coords.correctness, 0);
    assert!((coords.latitude - 10.0).abs() < 1e-6);
    assert!((coords.longitude - 20.0).abs() < 1e-6);
}

#[test]
fn test_parse_speed_direction() {
    let packet = common::parse_main_packet();
    let sd = packet.tags.speed_direction.as_ref().expect("no speed_direction");
    assert_eq!(sd.speed_kmh, 100); // 1000 / 10 = 100
    assert!((sd.direction_deg - 90.0).abs() < 0.1); // 900 / 10 = 90.0
}

#[test]
fn test_parse_altitude_and_hdop() {
    let packet = common::parse_main_packet();
    assert_eq!(packet.tags.altitude, Some(100));
    assert_eq!(packet.tags.hdop, Some(10)); // raw; normalized to 1.0 by ÷10
}

#[test]
fn test_parse_too_short() {
    assert!(parse_packet(&[0x01, 0x02]).is_err()); // only 2 bytes, need at least 3
    assert!(parse_packet(&[]).is_err());
}

#[test]
fn test_parse_tag_len_exceeds_data() {
    // Declare tag_len = 100, but provide 0 tag bytes.
    let frame = [0x01u8, 0x64, 0x00]; // raw_len = 100
    assert!(parse_packet(&frame).is_err());
}
