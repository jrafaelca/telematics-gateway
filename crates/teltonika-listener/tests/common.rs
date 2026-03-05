#![allow(dead_code)]

use teltonika_listener::crc::crc16_ibm;

pub fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Codec 8 example packet from the Teltonika wiki.
///
/// Breakdown:
/// - `00000000`          preamble
/// - `00000036`          data_field_length = 54
/// - `08`                Codec 8
/// - `01`                num_data_1 = 1
/// - AVL record (52 bytes):
///   - `0000016B40D8EA30` timestamp = 1 560 928 388 656 ms
///   - `01`              priority = 1
///   - `00000000`        longitude = 0 (no fix)
///   - `00000000`        latitude = 0 (no fix)
///   - `0000`            altitude = 0
///   - `0000`            angle = 0
///   - `00`              satellites = 0
///   - `0000`            speed = 0
///   - `01`              event_io_id = 1 (IO element #1 triggered this record)
///   - `05`              n_total = 5
///   - `02`              n1 = 2: (0x15, 0x03), (0x01, 0x01)
///   - `01`              n2 = 1: (0x42, 0x5E0F)
///   - `01`              n4 = 1: (0xF1, 0x0000601A)
///   - `01`              n8 = 1: (0x4E, 0x0000000000000000)
/// - `01`                num_data_2 = 1
/// - `0000C7CF`          CRC-16/IBM = 0xC7CF
pub const CODEC8_EXAMPLE_HEX: &str = concat!(
    "00000000",
    "00000036",
    "08", "01",
    "0000016B40D8EA30",
    "01",
    "00000000",
    "00000000",
    "0000",
    "0000",
    "00",
    "0000",
    "01",
    "05",
    "02", "1503", "0101",
    "01", "42", "5E0F",
    "01", "F1", "0000601A",
    "01", "4E", "0000000000000000",
    "01",
    "0000C7CF"
);

/// Builds a Codec 8 AVL packet with one GPS record at the given coordinates.
///
/// The caller controls IMEI externally (it is sent via the TCP handshake, not
/// embedded in AVL packets).
pub fn build_avl_packet(
    timestamp_ms: u64,
    lat_deg: f64,
    lon_deg: f64,
    altitude_m: i16,
    angle_deg: u16,
    satellites: u8,
    speed_kmh: u16,
) -> Vec<u8> {
    let lon_raw = (lon_deg * 10_000_000.0) as i32;
    let lat_raw = (lat_deg * 10_000_000.0) as i32;

    let mut record = Vec::new();
    record.extend_from_slice(&timestamp_ms.to_be_bytes());
    record.push(0x01u8); // priority = High
    record.extend_from_slice(&lon_raw.to_be_bytes());
    record.extend_from_slice(&lat_raw.to_be_bytes());
    record.extend_from_slice(&altitude_m.to_be_bytes());
    record.extend_from_slice(&angle_deg.to_be_bytes());
    record.push(satellites);
    record.extend_from_slice(&speed_kmh.to_be_bytes());
    // IO element: event_io_id=0, n_total=0, no IO groups.
    record.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    // Data field: codec_id + num_data_1 + record + num_data_2.
    let mut data_field = vec![0x08u8, 0x01u8];
    data_field.extend_from_slice(&record);
    data_field.push(0x01u8); // num_data_2

    let crc = crc16_ibm(&data_field) as u32;

    let dfl = data_field.len() as u32;
    let mut packet = vec![0x00u8, 0x00, 0x00, 0x00]; // preamble
    packet.extend_from_slice(&dfl.to_be_bytes());
    packet.extend_from_slice(&data_field);
    packet.extend_from_slice(&crc.to_be_bytes());

    packet
}
