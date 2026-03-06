//! Shared test fixtures for queclink-listener tests.

/// IMEI used across all test fixtures.
pub const IMEI: u64 = 864696060004173;

/// A well-formed `+RESP:GTFRI` line with:
/// - version = "060100"
/// - IMEI = 864696060004173
/// - device_name = "queclink"
/// - gnss_accuracy = 5 (HDOP 5.0, fix reported)
/// - speed = 50.5 km/h, azimuth = 22.3°, altitude = 250.0 m
/// - longitude = -2.6273°, latitude = -79.8418°
/// - gnss_utc_time = 20260305120000 (2026-03-05 12:00:00 UTC)
/// - position_append_mask = "01" (bit 0 = satellites present)
/// - satellites = 6
/// - send_time = 20260305120020
/// - count = 0001
pub const GTFRI_LINE: &str = concat!(
    "+RESP:GTFRI,060100,864696060004173,queclink,,0,1,",
    "5,50.5,22.3,250.0,-2.6273,-79.8418,20260305120000,",
    "0730,0002,68C7,5D5A,01,6,20260305120020,0001$\r\n"
);

/// A `+RESP:GTFRI` line with gnss_accuracy = 0 (no fix).
pub const GTFRI_NO_FIX_LINE: &str = concat!(
    "+RESP:GTFRI,060100,864696060004173,queclink,,0,1,",
    "0,0.0,0.0,0.0,0.0,0.0,20260305120000,",
    "0730,0002,68C7,5D5A,00,20260305120020,0000$\r\n"
);

/// A `+ACK:GTHBD` heartbeat line.
#[allow(dead_code)]
pub const GTHBD_LINE: &str =
    "+ACK:GTHBD,060100,864696060004173,queclink,20260305120000,0002$\r\n";

/// An unknown / unrecognised line.
#[allow(dead_code)]
pub const UNKNOWN_LINE: &str = "+EVT:GTPNA,060100,864696060004173$\r\n";
