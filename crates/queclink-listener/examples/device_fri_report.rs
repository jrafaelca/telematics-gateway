//! Device simulator — sends a `+RESP:GTFRI` GPS report and reads the server SACK.
//!
//! # Usage
//!
//! 1. Start the listener (with Valkey running):
//!    ```bash
//!    docker compose up valkey
//!    cargo run -p queclink-listener
//!    ```
//! 2. Run this example:
//!    ```bash
//!    cargo run -p queclink-listener --example device_fri_report
//!    ```
//!
//! After running, verify in Valkey:
//! ```bash
//! redis-cli HGETALL devices:864696060004173
//! redis-cli XLEN devices:records:$(redis-cli KEYS 'devices:records:*' | head -1)
//! ```

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const IMEI: u64 = 864696060004173;

/// GTFRI line with a valid GPS fix (gnss_accuracy = 5, satellites = 6).
///
/// Fields:
/// - version = 060100
/// - IMEI = 864696060004173
/// - device_name = queclink
/// - gnss_accuracy = 5 (HDOP 5.0)
/// - speed = 50.5 km/h, azimuth = 22.3°, altitude = 250.0 m
/// - longitude = -2.6273°, latitude = -79.8418° (Cuenca, Ecuador)
/// - position_append_mask = 01 (bit 0 = satellites field present)
/// - satellites = 6
const GTFRI_LINE: &str = concat!(
    "+RESP:GTFRI,060100,864696060004173,queclink,,0,1,",
    "5,50.5,22.3,250.0,-2.6273,-79.8418,20260305120000,",
    "0730,0002,68C7,5D5A,01,6,20260305120020,0001$\r\n"
);

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7950";

    println!("=== device_fri_report ===");
    println!("IMEI: {IMEI}");
    println!();

    println!("Connecting to {addr}...");
    let stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect — is queclink-listener running on port 7950?");
    println!("Connected.");
    println!();

    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);

    // Send GTFRI GPS report.
    println!("Sending GTFRI report...");
    println!("  {}", GTFRI_LINE.trim_end());
    writer
        .write_all(GTFRI_LINE.as_bytes())
        .await
        .expect("send failed");

    // Read server SACK.
    let mut sack = String::new();
    match reader.read_line(&mut sack).await {
        Ok(0) => eprintln!("Server closed connection without SACK."),
        Ok(_) => println!("SACK received: {}", sack.trim_end()),
        Err(e) => eprintln!("Error reading SACK: {e}"),
    }

    println!();
    println!("Verify in Valkey:");
    println!("  redis-cli HGETALL devices:{IMEI}");
}
