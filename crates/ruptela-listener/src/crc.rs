//! CRC-CCITT Kermit checksum used by the Ruptela protocol.
//!
//! The checksum is computed over the packet **body only** (the 2-byte length
//! header and the 2-byte CRC trailer are excluded). Polynomial: `0x8408`
//! (bit-reversed representation of `0x1021`), initial value: `0x0000`.

/// Computes a CRC-CCITT Kermit checksum over `data`.
///
/// # Examples
///
/// ```
/// // From the Ruptela spec §3.2.1: body [0x64, 0x01] → CRC 0x13BC
/// assert_eq!(ruptela_listener::crc::crc16(&[0x64, 0x01]), 0x13BC);
/// ```
pub fn crc16(data: &[u8]) -> u16 {
    let poly: u16 = 0x8408; // bit-reversed 0x1021
    let mut crc: u16 = 0;
    for byte in data {
        crc ^= *byte as u16;
        for _ in 0..8 {
            let carry = crc & 1;
            crc >>= 1;
            if carry != 0 {
                crc ^= poly;
            }
        }
    }
    crc
}
