mod common;

use queclink_listener::protocol::{parse_line, Message, parse_datetime};

#[test]
fn test_parse_fri_line() {
    let msg = parse_line(common::GTFRI_LINE);
    match msg {
        Message::FriReport(rec) => {
            assert_eq!(rec.imei, common::IMEI);
            assert_eq!(rec.version, "060100");
            assert_eq!(rec.device_name, "queclink");
            assert_eq!(rec.msg_type, "FRI");
            assert!((rec.gnss_accuracy - 5.0).abs() < 1e-9);
            assert!((rec.speed - 50.5).abs() < 1e-9);
            assert!((rec.azimuth - 22.3).abs() < 1e-9);
            assert!((rec.altitude - 250.0).abs() < 1e-9);
            assert!((rec.longitude - (-2.6273)).abs() < 1e-6);
            assert!((rec.latitude - (-79.8418)).abs() < 1e-6);
            assert_eq!(rec.satellites, 6); // from optional field (mask bit 0)
            assert!((rec.hdop - 5.0).abs() < 1e-9); // falls back to gnss_accuracy
            assert_eq!(rec.count, "0001");
        }
        other => panic!("expected FriReport, got {other:?}"),
    }
}

#[test]
fn test_parse_fri_no_fix() {
    let msg = parse_line(common::GTFRI_NO_FIX_LINE);
    match msg {
        Message::FriReport(rec) => {
            assert_eq!(rec.gnss_accuracy, 0.0);
            assert_eq!(rec.satellites, 0); // no fix → 0 satellites
        }
        other => panic!("expected FriReport, got {other:?}"),
    }
}

#[test]
fn test_parse_heartbeat() {
    let msg = parse_line(common::GTHBD_LINE);
    match msg {
        Message::Heartbeat(hbd) => {
            assert_eq!(hbd.imei, common::IMEI);
            assert_eq!(hbd.version, "060100");
            assert_eq!(hbd.device_name, "queclink");
            assert_eq!(hbd.count, "0002");
        }
        other => panic!("expected Heartbeat, got {other:?}"),
    }
}

#[test]
fn test_parse_unknown() {
    let msg = parse_line(common::UNKNOWN_LINE);
    assert!(matches!(msg, Message::Unknown));
}

#[test]
fn test_parse_command_ack() {
    let line = "+ACK:GTRTO,060100,864696060004173,queclink,GPS,0001,20260305120020,0001$\r\n";
    let msg = parse_line(line);
    match msg {
        Message::CommandAck(ack) => {
            assert_eq!(ack.msg_type, "RTO");
            assert_eq!(ack.imei, common::IMEI);
            assert_eq!(ack.serial_num, "0001");
            assert_eq!(ack.count, "0001");
        }
        other => panic!("expected CommandAck, got {other:?}"),
    }
}

#[test]
fn test_parse_datetime() {
    // 2026-03-05 12:00:00 UTC
    // 2025-01-01 00:00:00 = 1735689600 (verified reference)
    // 2026-01-01 = 1735689600 + 365*86400 = 1767225600
    // 2026-03-05 = 1767225600 + (31+28+4)*86400 = 1767225600 + 5443200 = 1772668800
    // + 12h (43200) = 1772712000
    let ts = parse_datetime("20260305120000").unwrap();
    assert_eq!(ts, 1772712000);
}

#[test]
fn test_parse_datetime_invalid() {
    assert!(parse_datetime("").is_none());
    assert!(parse_datetime("2026030512000").is_none()); // 13 chars
    assert!(parse_datetime("202603051200000").is_none()); // 15 chars
}

#[test]
fn test_parse_buff_gtfri() {
    let line = concat!(
        "+BUFF:GTFRI,060100,864696060004173,queclink,,0,1,",
        "3,30.0,180.0,100.0,10.0,20.0,20260101000000,",
        "0000,0000,0000,0000,00,20260101000010,ABCD$\r\n"
    );
    let msg = parse_line(line);
    assert!(matches!(msg, Message::FriReport(_)));
}

#[test]
fn test_parse_location_report() {
    // GTTOW — generic location with same field layout as GTFRI
    let line = concat!(
        "+RESP:GTTOW,060100,864696060004173,queclink,,0,1,",
        "3,30.0,180.0,100.0,10.0,20.0,20260101000000,",
        "0000,0000,0000,0000,00,20260101000010,ABCD$\r\n"
    );
    let msg = parse_line(line);
    assert!(matches!(msg, Message::LocationReport(_)));
}
