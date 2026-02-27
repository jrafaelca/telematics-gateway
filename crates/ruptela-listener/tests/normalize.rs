mod common;

use ruptela_listener::normalize::normalize;
use ruptela_listener::protocol::{IoElement, Record};
use shared::normalize::stream_key;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_record() -> Record {
    Record {
        timestamp: 1_400_000_000,
        timestamp_ext: 0,
        record_extension: None,
        priority: 1,
        longitude: 42.123456,
        latitude: -18.654321,
        altitude: 123.4,
        angle: 270.0,
        satellites: 8,
        speed: 55,
        hdop: 1.2,
        event_id: 7,
        io: vec![],
    }
}

// ── normalize() ───────────────────────────────────────────────────────────────

#[test]
fn test_normalize_field_mapping() {
    let record = make_record();
    let nr = normalize(13_226_005_504_143, &record, 1_700_000_000_000);

    assert_eq!(nr.imei, 13_226_005_504_143);
    assert_eq!(nr.received_at, 1_700_000_000_000);
    assert_eq!(nr.timestamp, 1_400_000_000);
    assert!((nr.longitude - 42.123456).abs() < 1e-9);
    assert!((nr.latitude - (-18.654321)).abs() < 1e-9);
    assert!((nr.altitude - 123.4).abs() < 0.01);
    assert!((nr.angle - 270.0).abs() < 0.01);
    assert_eq!(nr.satellites, 8);
    assert_eq!(nr.speed, 55);
    assert!((nr.hdop - 1.2).abs() < 0.01);
}

#[test]
fn test_normalize_can_data_is_empty_object() {
    let record = make_record();
    let nr = normalize(1, &record, 0);
    assert_eq!(nr.can_data, serde_json::json!({}));
}

#[test]
fn test_normalize_with_io_elements_does_not_panic() {
    let mut record = make_record();
    record.io = vec![
        IoElement { id: 0x01, value: 100 },
        IoElement { id: 0x02, value: 200 },
    ];
    // normalize() currently ignores IO — just ensure it doesn't panic.
    let nr = normalize(99, &record, 42);
    assert_eq!(nr.imei, 99);
}

#[test]
fn test_normalize_from_real_packet() {
    let data = common::hex(common::RECORDS_PACKET_HEX);
    let packet_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let body = &data[2..2 + packet_len];

    let packet = ruptela_listener::protocol::parse_packet(body).expect("parse_packet failed");
    let records = match packet.payload {
        ruptela_listener::protocol::Payload::Records { records, .. } => records,
        _ => panic!("Expected Payload::Records"),
    };

    let nr = normalize(packet.imei, &records[0], 9_999_999);
    assert_eq!(nr.imei, 13_226_005_504_143);
    assert_eq!(nr.timestamp, 0x5268CEF2);
    assert!((nr.longitude - 42.6654266).abs() < 1e-6);
    assert!((nr.latitude - 18.3451283).abs() < 1e-6);
}

// ── NormalizedRecord::to_fields() ─────────────────────────────────────────────

#[test]
fn test_to_fields_key_names() {
    let record = make_record();
    let nr = normalize(1, &record, 0);
    let fields = nr.to_fields();
    let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();

    assert_eq!(
        keys,
        &[
            "imei", "received_at", "timestamp", "longitude", "latitude",
            "altitude", "angle", "satellites", "speed", "hdop", "can_data",
        ]
    );
}

#[test]
fn test_to_fields_precision() {
    let record = make_record();
    let nr = normalize(1, &record, 0);
    let fields: std::collections::HashMap<_, _> = nr.to_fields().into_iter().collect();

    // longitude and latitude: 6 decimal places
    assert_eq!(fields["longitude"], "42.123456");
    assert_eq!(fields["latitude"], "-18.654321");
    // altitude: 1 decimal place
    assert_eq!(fields["altitude"], "123.4");
    // angle: 2 decimal places
    assert_eq!(fields["angle"], "270.00");
    // hdop: 1 decimal place
    assert_eq!(fields["hdop"], "1.2");
}

#[test]
fn test_to_fields_scalar_values() {
    let record = make_record();
    let nr = normalize(13_226_005_504_143, &record, 1_700_000_000_000);
    let fields: std::collections::HashMap<_, _> = nr.to_fields().into_iter().collect();

    assert_eq!(fields["imei"], "13226005504143");
    assert_eq!(fields["received_at"], "1700000000000");
    assert_eq!(fields["timestamp"], "1400000000");
    assert_eq!(fields["satellites"], "8");
    assert_eq!(fields["speed"], "55");
    assert_eq!(fields["can_data"], "{}");
}

// ── stream_key() ──────────────────────────────────────────────────────────────

#[test]
fn test_stream_key_sharding() {
    // IMEI 10 with 3 shards → 10 % 3 = 1
    assert_eq!(stream_key(10, 3), "devices:records:1");
    // IMEI 9 with 3 shards → 9 % 3 = 0
    assert_eq!(stream_key(9, 3), "devices:records:0");
    // IMEI 11 with 3 shards → 11 % 3 = 2
    assert_eq!(stream_key(11, 3), "devices:records:2");
}

#[test]
fn test_stream_key_single_shard() {
    // Any IMEI with 1 shard always maps to shard 0.
    assert_eq!(stream_key(13_226_005_504_143, 1), "devices:records:0");
    assert_eq!(stream_key(0, 1), "devices:records:0");
}

#[test]
fn test_stream_key_format() {
    let key = stream_key(13_226_005_504_143, 8);
    assert!(key.starts_with("devices:records:"));
    let shard: u64 = key.trim_start_matches("devices:records:").parse().unwrap();
    assert_eq!(shard, 13_226_005_504_143 % 8);
}
