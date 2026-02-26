mod common;
use ruptela_listener::protocol::{parse_packet, Payload};

#[test]
fn test_parse_packet_metadata() {
    let data = common::hex(common::RECORDS_PACKET_HEX);
    let packet_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let body = &data[2..2 + packet_len];

    let packet = parse_packet(body).expect("parse_packet failed");

    assert_eq!(packet.imei, 13226005504143);
    assert_eq!(packet.command_id, 0x01);

    match packet.payload {
        Payload::Records { records_left, num_records, ref records } => {
            assert_eq!(records_left, 1);
            assert_eq!(num_records, 30);
            assert_eq!(records.len(), 30);
        }
        _ => panic!("Expected Payload::Records"),
    }
}

#[test]
fn test_parse_first_record_gps() {
    let data = common::hex(common::RECORDS_PACKET_HEX);
    let packet_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let body = &data[2..2 + packet_len];

    let packet = parse_packet(body).expect("parse_packet failed");

    let records = match packet.payload {
        Payload::Records { records, .. } => records,
        _ => panic!("Expected Payload::Records"),
    };

    let r = &records[0];
    assert_eq!(r.timestamp, 0x5268CEF2);
    assert_eq!(r.timestamp_ext, 0);
    assert_eq!(r.priority, 0);
    assert!((r.longitude - 42.6654266).abs() < 1e-6);
    assert!((r.latitude - 18.3451283).abs() < 1e-6);
    assert_eq!(r.event_id, 7);
    assert!(r.io.is_empty());
}

#[test]
fn test_parse_packet_too_short() {
    let result = parse_packet(&[0x00, 0x01, 0x02]);
    assert!(result.is_err());
}

#[test]
fn test_parse_extended_packet_metadata() {
    let packet = common::parse_extended_packet();
    assert_eq!(packet.command_id, 0x44);
    match packet.payload {
        Payload::ExtendedRecords { records_left, num_records, ref records } => {
            assert_eq!(records_left, 0);
            assert_eq!(num_records, 2);
            assert_eq!(records.len(), 2);
        }
        _ => panic!("Expected Payload::ExtendedRecords"),
    }
}

#[test]
fn test_parse_extended_first_record() {
    let packet = common::parse_extended_packet();
    let records = match packet.payload {
        Payload::ExtendedRecords { records, .. } => records,
        _ => panic!("Expected Payload::ExtendedRecords"),
    };

    let r = &records[0];
    assert_eq!(r.timestamp, 0x698a750a);
    assert_eq!(r.record_extension, Some(0x10));
    assert_eq!(r.event_id, 0x0009);
    assert_eq!(r.satellites, 0x15);
    assert_eq!(r.speed, 31);
    assert!((r.longitude - (-69.047585)).abs() < 1e-5);
    assert!((r.latitude - (-22.316970)).abs() < 1e-5);
    // 11 (1B) + 7 (2B) + 6 (4B) + 0 (8B) = 24 IO elements
    assert_eq!(r.io.len(), 24);
    assert_eq!(r.io[0].id, 0x0199);
    assert_eq!(r.io[0].value, 1);
}

#[test]
fn test_parse_extended_second_record_io() {
    let packet = common::parse_extended_packet();
    let records = match packet.payload {
        Payload::ExtendedRecords { records, .. } => records,
        _ => panic!("Expected Payload::ExtendedRecords"),
    };

    let r = &records[1];
    assert_eq!(r.record_extension, Some(0x11));
    assert_eq!(r.event_id, 0x0009);
    // 6 (1B) + 1 (2B) + 2 (4B) + 4 (8B) = 13 IO elements
    assert_eq!(r.io.len(), 13);
    let last = r.io.last().unwrap();
    assert_eq!(last.id, 0x0481);
    assert_eq!(last.value, u64::MAX);
}
