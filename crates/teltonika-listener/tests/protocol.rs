mod common;
use teltonika_listener::protocol::{IoValue, parse_packet};

#[test]
fn test_parse_codec8_example_record_count() {
    let full = common::hex(common::CODEC8_EXAMPLE_HEX);
    let data_field = &full[8..full.len() - 4];
    let packet = parse_packet(data_field).expect("parse failed");
    assert_eq!(packet.codec_id, 0x08);
    assert_eq!(packet.records.len(), 1);
}

#[test]
fn test_parse_codec8_example_gps() {
    let full = common::hex(common::CODEC8_EXAMPLE_HEX);
    let data_field = &full[8..full.len() - 4];
    let packet = parse_packet(data_field).unwrap();
    let r = &packet.records[0];
    assert_eq!(r.timestamp_ms, 0x0000016B40D8EA30);
    assert_eq!(r.priority, 1);
    assert_eq!(r.longitude, 0.0);
    assert_eq!(r.latitude, 0.0);
    assert_eq!(r.altitude, 0);
    assert_eq!(r.angle, 0);
    assert_eq!(r.satellites, 0);
    assert_eq!(r.speed, 0);
}

#[test]
fn test_parse_codec8_example_io_elements() {
    let full = common::hex(common::CODEC8_EXAMPLE_HEX);
    let data_field = &full[8..full.len() - 4];
    let packet = parse_packet(data_field).unwrap();
    let r = &packet.records[0];
    assert_eq!(r.event_io_id, 1);
    assert_eq!(r.io_elements.len(), 5);

    // N1 elements: (0x15=21, 3) and (0x01=1, 1)
    assert_eq!(r.io_elements[0].id, 0x15);
    assert!(matches!(r.io_elements[0].value, IoValue::U8(3)));
    assert_eq!(r.io_elements[1].id, 0x01);
    assert!(matches!(r.io_elements[1].value, IoValue::U8(1)));

    // N2 element: (0x42=66, 0x5E0F)
    assert_eq!(r.io_elements[2].id, 0x42);
    assert!(matches!(r.io_elements[2].value, IoValue::U16(0x5E0F)));

    // N4 element: (0xF1=241, 0x0000601A)
    assert_eq!(r.io_elements[3].id, 0xF1);
    assert!(matches!(r.io_elements[3].value, IoValue::U32(0x0000601A)));

    // N8 element: (0x4E=78, 0)
    assert_eq!(r.io_elements[4].id, 0x4E);
    assert!(matches!(r.io_elements[4].value, IoValue::U64(0)));
}

#[test]
fn test_parse_gps_record_coordinates() {
    // Build a packet with known coordinates: lat=54.1234567°, lon=25.7654321°.
    let packet_bytes = common::build_avl_packet(
        1_000_000_000_000,
        54.1234567,
        25.7654321,
        150,
        45,
        8,
        60,
    );
    let data_field = &packet_bytes[8..packet_bytes.len() - 4];
    let packet = parse_packet(data_field).unwrap();
    let r = &packet.records[0];
    assert_eq!(r.satellites, 8);
    assert_eq!(r.speed, 60);
    assert!((r.latitude - 54.1234567).abs() < 1e-6);
    assert!((r.longitude - 25.7654321).abs() < 1e-6);
    assert_eq!(r.altitude, 150);
    assert_eq!(r.angle, 45);
}

#[test]
fn test_parse_too_short() {
    assert!(parse_packet(&[]).is_err());
    assert!(parse_packet(&[0x08]).is_err());
    assert!(parse_packet(&[0x08, 0x01]).is_err());
}

#[test]
fn test_parse_unsupported_codec() {
    let data = [0x0C, 0x01, 0x01]; // Codec 12 is not supported
    assert!(parse_packet(&data).is_err());
}

#[test]
fn test_parse_num_data_mismatch() {
    // Build a valid Codec 8 packet body but force num_data_2 to differ from num_data_1.
    let packet_bytes = common::build_avl_packet(1_000_000_000_000, 10.0, 20.0, 0, 0, 5, 50);
    let mut data_field: Vec<u8> = packet_bytes[8..packet_bytes.len() - 4].to_vec();
    // Corrupt the last byte (num_data_2).
    *data_field.last_mut().unwrap() = 0xFF;
    assert!(parse_packet(&data_field).is_err());
}
