mod common;
use teltonika_listener::crc::crc16_ibm;

#[test]
fn test_crc_empty() {
    assert_eq!(crc16_ibm(&[]), 0x0000);
}

#[test]
fn test_crc_single_byte() {
    // CRC-16/IBM of [0x01] = 0xC0C1
    assert_eq!(crc16_ibm(&[0x01]), 0xC0C1);
}

#[test]
fn test_crc_codec8_example() {
    // Data field of the Teltonika wiki Codec 8 example packet → CRC = 0xC7CF.
    let full = common::hex(common::CODEC8_EXAMPLE_HEX);
    // Data field starts at byte 8 (after 4B preamble + 4B dfl) and ends 4 bytes
    // before the end (CRC trailer).
    let data_field = &full[8..full.len() - 4];
    assert_eq!(crc16_ibm(data_field), 0xC7CF);
}

#[test]
fn test_crc_differs_from_modbus() {
    // CRC-16/IBM (init 0x0000) must differ from MODBUS (init 0xFFFF) for
    // non-empty input.
    let data = b"teltonika";
    let ibm = crc16_ibm(data);
    // MODBUS inline for comparison.
    let modbus = {
        let poly: u16 = 0xA001;
        let mut crc: u16 = 0xFFFF;
        for &b in data {
            crc ^= b as u16;
            for _ in 0..8 {
                if crc & 1 != 0 { crc = (crc >> 1) ^ poly; } else { crc >>= 1; }
            }
        }
        crc
    };
    assert_ne!(ibm, modbus);
}
