mod common;
use galileosky_listener::normalize::normalize;
use galileosky_listener::protocol::{Coordinates, SpeedDir, TagSet};

const IMEI: u64 = 861230043907626;
const RECEIVED_AT: u64 = 1_700_000_000_000;

fn tags_with_gps(correctness: u8) -> TagSet {
    TagSet {
        imei: Some(IMEI),
        timestamp: Some(1_000_000),
        coordinates: Some(Coordinates {
            satellites: 8,
            correctness,
            latitude: 10.0,
            longitude: 20.0,
        }),
        speed_direction: Some(SpeedDir {
            speed_kmh: 100,
            direction_deg: 90.0,
        }),
        altitude: Some(150),
        hdop: Some(12), // raw; 1.2 after ÷10
        ..Default::default()
    }
}

#[test]
fn test_normalize_valid_gps() {
    let tags = tags_with_gps(0); // correctness = 0 (GPS valid)
    let rec = normalize(IMEI, &tags, RECEIVED_AT).expect("expected Some");
    assert_eq!(rec.imei, IMEI);
    assert_eq!(rec.received_at, RECEIVED_AT);
    assert_eq!(rec.timestamp, 1_000_000);
}

#[test]
fn test_normalize_valid_cell() {
    let tags = tags_with_gps(2); // correctness = 2 (cell-based valid)
    assert!(normalize(IMEI, &tags, RECEIVED_AT).is_some());
}

#[test]
fn test_normalize_invalid_correctness() {
    let tags = tags_with_gps(1); // correctness = 1 → invalid
    assert!(normalize(IMEI, &tags, RECEIVED_AT).is_none());

    let tags = tags_with_gps(5);
    assert!(normalize(IMEI, &tags, RECEIVED_AT).is_none());
}

#[test]
fn test_normalize_missing_timestamp() {
    let mut tags = tags_with_gps(0);
    tags.timestamp = None;
    assert!(normalize(IMEI, &tags, RECEIVED_AT).is_none());
}

#[test]
fn test_normalize_missing_coordinates() {
    let mut tags = tags_with_gps(0);
    tags.coordinates = None;
    assert!(normalize(IMEI, &tags, RECEIVED_AT).is_none());
}

#[test]
fn test_normalize_field_values() {
    let tags = tags_with_gps(0);
    let rec = normalize(IMEI, &tags, RECEIVED_AT).unwrap();
    assert!((rec.latitude - 10.0).abs() < 1e-9);
    assert!((rec.longitude - 20.0).abs() < 1e-9);
    assert_eq!(rec.altitude, 150.0);
    assert_eq!(rec.speed, 100);
    assert!((rec.angle - 90.0).abs() < 0.01);
    assert_eq!(rec.satellites, 8);
    assert!((rec.hdop - 1.2).abs() < 0.001);
}

#[test]
fn test_normalize_can_data_empty() {
    let tags = tags_with_gps(0);
    let rec = normalize(IMEI, &tags, RECEIVED_AT).unwrap();
    assert_eq!(rec.can_data, serde_json::json!({}));
}

#[test]
fn test_normalize_missing_optional_fields() {
    // Without speed/direction and altitude/hdop, normalize still succeeds with defaults.
    let tags = TagSet {
        imei: Some(IMEI),
        timestamp: Some(1_000_000),
        coordinates: Some(Coordinates {
            satellites: 5,
            correctness: 0,
            latitude: 5.0,
            longitude: 10.0,
        }),
        ..Default::default()
    };
    let rec = normalize(IMEI, &tags, RECEIVED_AT).unwrap();
    assert_eq!(rec.speed, 0);
    assert_eq!(rec.angle, 0.0);
    assert_eq!(rec.altitude, 0.0);
    assert_eq!(rec.hdop, 0.0);
}
