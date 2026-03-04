mod common;
use galileosky_listener::crc::crc16_modbus;

#[test]
fn test_crc16_modbus_empty() {
    // CRC of empty input: init value 0xFFFF unchanged.
    assert_eq!(crc16_modbus(&[]), 0xFFFF);
}

#[test]
fn test_crc16_modbus_known_vector() {
    // Head packet from the Galileosky specification (page 5), without the
    // 2-byte CRC trailer.  The expected CRC is 0x298F (stored LE as 8F 29).
    let data = common::hex(common::HEAD_PACKET_HEX);
    let without_crc = &data[..data.len() - 2];
    assert_eq!(crc16_modbus(without_crc), 0x298F);
}

#[test]
fn test_crc16_modbus_main_packet() {
    // Main packet is built with the correct CRC appended; verify it round-trips.
    let data = common::build_main_packet();
    let without_crc = &data[..data.len() - 2];
    let crc_in_packet = u16::from_le_bytes([data[data.len() - 2], data[data.len() - 1]]);
    assert_eq!(crc16_modbus(without_crc), crc_in_packet);
}
