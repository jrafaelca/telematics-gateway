//! CRC-16/IBM checksum used by the Teltonika protocol.
//!
//! The checksum is computed over the **data field** of the packet — from the
//! Codec ID byte through and including the Number of Data 2 byte.  The 4-byte
//! CRC field appended after the data field is excluded from the computation.
//!
//! Algorithm: CRC-16/IBM — polynomial `0x8005` (bit-reversed representation
//! `0xA001`), initial value `0x0000`, reflected input and output.
//!
//! This differs from CRC-16/MODBUS only in the initial value (`0x0000` vs
//! `0xFFFF`).

/// Computes a CRC-16/IBM checksum over `data`.
///
/// # Examples
///
/// ```
/// // Codec 8 example from the Teltonika wiki (data field only, 54 bytes) → 0xC7CF
/// let data: Vec<u8> = vec![
///     0x08, 0x01,
///     0x00, 0x00, 0x01, 0x6B, 0x40, 0xD8, 0xEA, 0x30,
///     0x01,
///     0x00, 0x00, 0x00, 0x00,
///     0x00, 0x00, 0x00, 0x00,
///     0x00, 0x00,
///     0x00, 0x00,
///     0x00,
///     0x00, 0x00,
///     0x01, 0x05,
///     0x02,
///     0x15, 0x03,
///     0x01, 0x01,
///     0x01,
///     0x42, 0x5E, 0x0F,
///     0x01,
///     0xF1, 0x00, 0x00, 0x60, 0x1A,
///     0x01,
///     0x4E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
///     0x01,
/// ];
/// assert_eq!(teltonika_listener::crc::crc16_ibm(&data), 0xC7CF);
/// ```
pub fn crc16_ibm(data: &[u8]) -> u16 {
    let poly: u16 = 0xA001;
    let mut crc: u16 = 0x0000;
    for byte in data {
        crc ^= *byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ poly;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}
