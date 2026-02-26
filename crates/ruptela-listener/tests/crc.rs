mod common;
use ruptela_listener::crc::crc16;

#[test]
fn test_crc16_ack_body() {
    // Del spec sección 3.2.1: body=[0x64, 0x01] → CRC=0x13BC
    assert_eq!(crc16(&[0x64, 0x01]), 0x13BC);
}

#[test]
fn test_crc16_records_packet() {
    // Del spec sección 3.2.1: CRC del body de 821 bytes = 0x46E2
    let data = common::hex(common::RECORDS_PACKET_HEX);
    let packet_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let body = &data[2..2 + packet_len];
    assert_eq!(crc16(body), 0x46E2);
}

#[test]
fn test_crc16_extended_records_packet() {
    let data = common::hex(common::EXTENDED_RECORDS_PACKET_HEX);
    let packet_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let body = &data[2..2 + packet_len];
    assert_eq!(crc16(body), 0xFE20);
}
