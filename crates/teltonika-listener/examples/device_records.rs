//! Device simulator — sends the Teltonika IMEI handshake followed by a Codec 8
//! AVL packet containing two GPS records.
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
//!    cargo run -p teltonika-listener --example device_records
//!    ```
//!
//! After running, verify in Valkey:
//! ```bash
//! redis-cli HGETALL devices:356307042441013
//! redis-cli XLEN devices:records:$(redis-cli KEYS 'devices:records:*' | head -1)
//! ```

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

fn build_record(timestamp_ms: u64, lat_deg: f64, lon_deg: f64, altitude: i16, angle: u16, satellites: u8, speed: u16) -> Vec<u8> {
    let lon_raw = (lon_deg * 10_000_000.0) as i32;
    let lat_raw = (lat_deg * 10_000_000.0) as i32;

    let mut r = Vec::new();
    r.extend_from_slice(&timestamp_ms.to_be_bytes());
    r.push(0x01); // priority = High
    r.extend_from_slice(&lon_raw.to_be_bytes());
    r.extend_from_slice(&lat_raw.to_be_bytes());
    r.extend_from_slice(&altitude.to_be_bytes());
    r.extend_from_slice(&angle.to_be_bytes());
    r.push(satellites);
    r.extend_from_slice(&speed.to_be_bytes());
    // IO: event_io_id=0, n_total=0, all counts 0.
    r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    r
}

fn build_avl_packet(records: &[Vec<u8>]) -> Vec<u8> {
    let mut data_field = vec![0x08u8, records.len() as u8];
    for r in records {
        data_field.extend_from_slice(r);
    }
    data_field.push(records.len() as u8); // num_data_2

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

    println!("=== device_records (Codec 8) ===");
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

    // 2. Send AVL packet with 2 records.
    let ts = now_ms();
    let r1 = build_record(ts - 5_000, 40.7128, -74.0060, 15, 90, 8, 60);   // New York
    let r2 = build_record(ts,         40.7580, -73.9855, 20, 180, 9, 80);  // Times Square
    let packet = build_avl_packet(&[r1, r2]);

    println!("Sending AVL packet with 2 records ({} bytes)...", packet.len());
    println!("  Record 1: lat=40.7128 lon=-74.0060 (New York)");
    println!("  Record 2: lat=40.7580 lon=-73.9855 (Times Square)");
    stream.write_all(&packet).await.expect("send failed");

    let mut resp = [0u8; 4];
    stream.read_exact(&mut resp).await.expect("read failed");
    let accepted = u32::from_be_bytes(resp);
    println!("Server accepted: {accepted} records");
    println!();

    println!("Verify in Valkey:");
    println!("  redis-cli HGETALL devices:{IMEI}");
}
