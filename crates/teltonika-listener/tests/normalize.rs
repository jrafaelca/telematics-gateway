mod common;
use teltonika_listener::normalize::normalize;
use teltonika_listener::protocol::parse_packet;

const IMEI: u64 = 356307042441013;
const RECEIVED_AT: u64 = 1_700_000_000_000;

#[test]
fn test_normalize_valid_gps_record() {
    let packet_bytes = common::build_avl_packet(
        1_000_000_000_000, // 1 000 000 000 s
        40.7128,
        -74.0060,
        10,
        90,
        8,
        60,
    );
    let data_field = &packet_bytes[8..packet_bytes.len() - 4];
    let packet = parse_packet(data_field).unwrap();
    let r = &packet.records[0];

    let rec = normalize(IMEI, r, RECEIVED_AT).expect("should normalize");
    assert_eq!(rec.imei, IMEI);
    assert_eq!(rec.received_at, RECEIVED_AT);
    assert_eq!(rec.timestamp, 1_000_000_000);
    assert!((rec.latitude - 40.7128).abs() < 1e-5);
    assert!((rec.longitude - (-74.0060)).abs() < 1e-5);
    assert_eq!(rec.altitude, 10.0);
    assert_eq!(rec.angle, 90.0);
    assert_eq!(rec.satellites, 8);
    assert_eq!(rec.speed, 60);
    assert_eq!(rec.hdop, 0.0);
}

#[test]
fn test_normalize_no_fix_satellites_zero() {
    // Satellites = 0 → normalize returns None.
    let packet_bytes = common::build_avl_packet(1_000_000_000_000, 0.0, 0.0, 0, 0, 0, 0);
    let data_field = &packet_bytes[8..packet_bytes.len() - 4];
    let packet = parse_packet(data_field).unwrap();
    let r = &packet.records[0];
    assert!(normalize(IMEI, r, RECEIVED_AT).is_none());
}

#[test]
fn test_normalize_null_island_rejected() {
    // lon=0 lat=0 with satellites>0 is still rejected (null island).
    let packet_bytes = common::build_avl_packet(1_000_000_000_000, 0.0, 0.0, 0, 0, 5, 0);
    let data_field = &packet_bytes[8..packet_bytes.len() - 4];
    let packet = parse_packet(data_field).unwrap();
    let r = &packet.records[0];
    assert!(normalize(IMEI, r, RECEIVED_AT).is_none());
}
