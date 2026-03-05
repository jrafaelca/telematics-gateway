//! Device simulator — sends a Codec 8 Extended (0x8E) AVL packet with IO elements.
//!
//! # Usage
//!
//! 1. Start the listener (with Valkey running):
//!    ```bash
//!    docker compose up valkey
//!    cargo run -p teltonika-listener
//!    ```
//! 2. Run this example:
//!    ```bash
//!    cargo run -p teltonika-listener --example device_extended_records
//!    ```

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const IMEI: u64 = 356307042441013;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn crc16_ibm(data: &[u8]) -> u16 {
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

/// Builds a Codec 8 Extended AVL packet with one record containing several IO elements.
fn build_extended_packet(timestamp_ms: u64, lat_deg: f64, lon_deg: f64) -> Vec<u8> {
    let lon_raw = (lon_deg * 10_000_000.0) as i32;
    let lat_raw = (lat_deg * 10_000_000.0) as i32;

    let mut record = Vec::new();

    // GPS fields.
    record.extend_from_slice(&timestamp_ms.to_be_bytes());
    record.push(0x01); // priority = High
    record.extend_from_slice(&lon_raw.to_be_bytes());
    record.extend_from_slice(&lat_raw.to_be_bytes());
    record.extend_from_slice(&100i16.to_be_bytes()); // altitude
    record.extend_from_slice(&270u16.to_be_bytes()); // angle (West)
    record.push(10u8); // satellites
    record.extend_from_slice(&120u16.to_be_bytes()); // speed = 120 km/h

    // IO element (Codec 8 Extended — all counts are 2 bytes, IDs are 2 bytes).
    // event_io_id = 0 (not event-driven)
    record.extend_from_slice(&0u16.to_be_bytes());
    // n_total = 3 (1 N1 + 1 N2 + 1 N4 + 0 N8 + 0 NX)
    record.extend_from_slice(&3u16.to_be_bytes());

    // N1 = 1: IO ID 0x00EF (ignition), value = 1 (on)
    record.extend_from_slice(&1u16.to_be_bytes());
    record.extend_from_slice(&0x00EFu16.to_be_bytes()); // id
    record.push(0x01u8); // value

    // N2 = 1: IO ID 0x00C8 (battery voltage), value = 4200 (mV)
    record.extend_from_slice(&1u16.to_be_bytes());
    record.extend_from_slice(&0x00C8u16.to_be_bytes()); // id
    record.extend_from_slice(&4200u16.to_be_bytes()); // value

    // N4 = 1: IO ID 0x00B7 (odometer), value = 123456 km
    record.extend_from_slice(&1u16.to_be_bytes());
    record.extend_from_slice(&0x00B7u16.to_be_bytes()); // id
    record.extend_from_slice(&123456u32.to_be_bytes()); // value

    // N8 = 0
    record.extend_from_slice(&0u16.to_be_bytes());
    // NX = 0
    record.extend_from_slice(&0u16.to_be_bytes());

    // Data field: codec_id=0x8E + num_data_1=1 + record + num_data_2=1.
    let mut data_field = vec![0x8Eu8, 0x01u8];
    data_field.extend_from_slice(&record);
    data_field.push(0x01u8);

    let crc = crc16_ibm(&data_field) as u32;
    let dfl = data_field.len() as u32;

    let mut packet = vec![0x00u8, 0x00, 0x00, 0x00]; // preamble
    packet.extend_from_slice(&dfl.to_be_bytes());
    packet.extend_from_slice(&data_field);
    packet.extend_from_slice(&crc.to_be_bytes());
    packet
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7900";

    println!("=== device_extended_records (Codec 8 Extended) ===");
    println!("IMEI: {IMEI}");
    println!();

    println!("Connecting to {addr}...");
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect — is teltonika-listener running on port 7900?");
    println!("Connected.");
    println!();

    // 1. IMEI handshake.
    let imei_str = format!("{IMEI:015}");
    let imei_bytes = imei_str.as_bytes();
    let mut handshake = Vec::new();
    handshake.extend_from_slice(&(imei_bytes.len() as u16).to_be_bytes());
    handshake.extend_from_slice(imei_bytes);
    println!("Sending IMEI handshake ({} bytes)...", handshake.len());
    stream.write_all(&handshake).await.expect("send failed");

    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack).await.expect("read failed");
    println!("IMEI response: 0x{:02X} ({})", ack[0], if ack[0] == 1 { "accepted" } else { "rejected" });
    if ack[0] != 1 {
        return;
    }
    println!();

    // 2. Send Codec 8 Extended packet with IO elements.
    let packet = build_extended_packet(now_ms(), 54.6872, 25.2797); // Vilnius, Lithuania
    println!("Sending Codec 8 Extended packet ({} bytes)...", packet.len());
    println!("  lat=54.6872 lon=25.2797 (Vilnius, Lithuania)");
    println!("  IO: ignition=1, battery=4200mV, odometer=123456km");
    stream.write_all(&packet).await.expect("send failed");

    let mut resp = [0u8; 4];
    stream.read_exact(&mut resp).await.expect("read failed");
    let accepted = u32::from_be_bytes(resp);
    println!("Server accepted: {accepted} records");
    println!();

    println!("Verify in Valkey:");
    println!("  redis-cli HGETALL devices:{IMEI}");
}
