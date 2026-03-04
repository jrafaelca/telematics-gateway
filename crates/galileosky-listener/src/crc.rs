//! CRC-16 MODBUS checksum used by the Galileosky protocol.
//!
//! The checksum is computed over the **entire packet** including header (1 B),
//! length field (2 B), and the tag section (L B).  The 2-byte CRC trailer is
//! excluded from the computation.  Polynomial: `0xA001` (bit-reversed
//! representation of `0x8005`), initial value: `0xFFFF`.

/// Computes a CRC-16 MODBUS checksum over `data`.
///
/// # Examples
///
/// ```
/// // Full head-packet from the Galileosky spec (without the 2-byte CRC trailer) → 0x298F
/// let pkt = galileosky_listener::crc::crc16_modbus(&[
///     0x01, 0x20, 0x00,
///     0x01, 0x9A, 0x02, 0x18,
///     0x03, 0x38, 0x36, 0x31, 0x32, 0x33, 0x30, 0x30, 0x34, 0x33, 0x39, 0x30, 0x37, 0x36, 0x32, 0x36,
///     0x04, 0x32, 0x00,
///     0xFE, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
/// ]);
/// assert_eq!(pkt, 0x298F);
/// ```
pub fn crc16_modbus(data: &[u8]) -> u16 {
    let poly: u16 = 0xA001;
    let mut crc: u16 = 0xFFFF;
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
