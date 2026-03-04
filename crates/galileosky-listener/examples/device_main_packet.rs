//! Device simulator — sends a HeadPack followed by a MainPack with GPS coordinates.
//!
//! # Usage
//!
//! 1. Start the listener (with Valkey running):
//!    ```bash
//!    docker compose up valkey
//!    cargo run -p galileosky-listener
//!    ```
//! 2. Run this example:
//!    ```bash
//!    cargo run -p galileosky-listener --example device_main_packet
//!    ```
//!
//! After running, verify the record was published:
//! ```bash
//! redis-cli XLEN devices:records:$(redis-cli KEYS 'devices:records:*' | head -1)
//! redis-cli HGETALL devices:861230043907626
//! ```

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const IMEI: u64 = 861230043907626;

/// Head packet from the Galileosky specification (page 5).
const HEAD_PACKET_HEX: &str = concat!(
    "01", "2000",
    "019A",
    "0218",
    "03", "383631323330303433393037363236",
    "04", "3200",
    "FE", "0600", "010000000000",
    "8F29"
);

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// CRC-16 MODBUS (poly 0xA001, init 0xFFFF).
fn crc16_modbus(data: &[u8]) -> u16 {
    let poly: u16 = 0xA001;
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        let mut b = byte;
        for _ in 0..8 {
            let mix = (crc ^ b as u16) & 0x01;
            crc >>= 1;
            if mix != 0 {
                crc ^= poly;
            }
            b >>= 1;
        }
    }
    crc
}

/// Builds a GPS main packet for the given IMEI and coordinates.
fn build_main_packet(imei: u64, timestamp: u32, lat_deg: f64, lon_deg: f64) -> Vec<u8> {
    let imei_str = format!("{imei:015}");
    let lat_raw = (lat_deg * 1_000_000.0) as i32;
    let lon_raw = (lon_deg * 1_000_000.0) as i32;

    let mut tags = Vec::new();

    // 0x03: IMEI
    tags.push(0x03u8);
    tags.extend_from_slice(imei_str.as_bytes());

    // 0x20: timestamp
    tags.push(0x20u8);
    tags.extend_from_slice(&timestamp.to_le_bytes());

    // 0x30: coordinates (9 bytes) — correctness=0, satellites=8
    tags.push(0x30u8);
    tags.push(0x08u8);
    tags.extend_from_slice(&lat_raw.to_le_bytes());
    tags.extend_from_slice(&lon_raw.to_le_bytes());

    // 0x33: speed=60 km/h (raw 600), direction=45.0° (raw 450)
    tags.push(0x33u8);
    tags.extend_from_slice(&600u16.to_le_bytes());
    tags.extend_from_slice(&450u16.to_le_bytes());

    // 0x34: altitude=250 m
    tags.push(0x34u8);
    tags.extend_from_slice(&250i16.to_le_bytes());

    // 0x35: hdop = 1.5 (raw 15)
    tags.extend_from_slice(&[0x35, 15]);

    let tag_len = tags.len() as u16;
    let mut frame = vec![0x01u8, (tag_len & 0xFF) as u8, (tag_len >> 8) as u8];
    frame.extend_from_slice(&tags);
    let crc = crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7800";

    println!("=== device_main_packet ===");
    println!("IMEI: {IMEI}");
    println!();

    println!("Connecting to {addr}...");
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect — is galileosky-listener running on port 7800?");
    println!("Connected.");
    println!();

    // 1. Send HeadPack.
    let head = from_hex(HEAD_PACKET_HEX);
    println!("Sending HeadPack ({} bytes)...", head.len());
    stream.write_all(&head).await.expect("send failed");

    let mut ack = [0u8; 3];
    stream.read_exact(&mut ack).await.expect("read ACK failed");
    println!("HeadPack ACK: {:02X?}", ack);
    println!();

    // 2. Send a GPS MainPack (New York coordinates).
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let main = build_main_packet(IMEI, now_ts, 40.7128, -74.0060);
    println!("Sending MainPack ({} bytes)...", main.len());
    println!("  ts={now_ts} lat=40.7128 lon=-74.0060 (New York)");
    stream.write_all(&main).await.expect("send failed");

    let mut ack = [0u8; 3];
    stream.read_exact(&mut ack).await.expect("read ACK failed");
    println!("MainPack ACK: {:02X?}", ack);
    println!();

    println!("Verify in Valkey:");
    println!("  redis-cli HGETALL devices:{IMEI}");
}
