mod common;

use queclink_listener::normalize::normalize;
use queclink_listener::protocol::{parse_line, Message};

fn fri_record_from(line: &str) -> queclink_listener::protocol::FriRecord {
    match parse_line(line) {
        Message::FriReport(rec) => rec,
        other => panic!("expected FriReport, got {other:?}"),
    }
}

const RECEIVED_AT: u64 = 1_741_132_800_000;

#[test]
fn test_normalize_field_mapping() {
    let rec = fri_record_from(common::GTFRI_LINE);
    let norm = normalize(common::IMEI, &rec, RECEIVED_AT).expect("normalize returned None");

    assert_eq!(norm.imei, common::IMEI);
    assert_eq!(norm.received_at, RECEIVED_AT);
    assert_eq!(norm.timestamp, rec.timestamp);
    assert!((norm.longitude - rec.longitude).abs() < 1e-6);
    assert!((norm.latitude - rec.latitude).abs() < 1e-6);
    assert!((norm.altitude as f64 - rec.altitude).abs() < 1e-3);
    assert!((norm.angle as f64 - rec.azimuth).abs() < 1e-3);
    assert_eq!(norm.satellites, rec.satellites);
    assert_eq!(norm.speed, rec.speed.round() as u16);
    assert!((norm.hdop as f64 - rec.hdop).abs() < 1e-3);
}

#[test]
fn test_normalize_no_fix_returns_none() {
    let rec = fri_record_from(common::GTFRI_NO_FIX_LINE);
    assert!(normalize(common::IMEI, &rec, RECEIVED_AT).is_none());
}

#[test]
fn test_normalize_null_island_returns_none() {
    let rec = fri_record_from(common::GTFRI_LINE);
    // Override coordinates to null island
    let mut null_rec = rec;
    null_rec.longitude = 0.0;
    null_rec.latitude = 0.0;
    assert!(normalize(common::IMEI, &null_rec, RECEIVED_AT).is_none());
}

#[test]
fn test_normalize_can_data_is_empty_object() {
    let rec = fri_record_from(common::GTFRI_LINE);
    let norm = normalize(common::IMEI, &rec, RECEIVED_AT).unwrap();
    assert_eq!(norm.can_data, serde_json::json!({}));
}
