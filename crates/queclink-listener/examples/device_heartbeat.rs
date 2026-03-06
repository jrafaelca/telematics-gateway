//! Device simulator — sends a `+ACK:GTHBD` heartbeat and reads the server SACK.
//!
//! Queclink devices send periodic heartbeats when there is no position data to
//! report.  The server replies with `+SACK:GTHBD,...$\r\n`.
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
//!    cargo run -p queclink-listener --example device_heartbeat
//!    ```

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const IMEI: u64 = 864696060004173;

/// Sends a GTFRI report first so the server learns the IMEI, then sends the HBD.
const GTFRI_LINE: &str = concat!(
    "+RESP:GTFRI,060100,864696060004173,queclink,,0,1,",
    "5,50.5,22.3,250.0,-2.6273,-79.8418,20260305120000,",
    "0730,0002,68C7,5D5A,01,6,20260305120020,0001$\r\n"
);

const GTHBD_LINE: &str =
    "+ACK:GTHBD,060100,864696060004173,queclink,20260305120010,0002$\r\n";

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7950";

    println!("=== device_heartbeat ===");
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

    // 1. Send GTFRI first so the server registers the IMEI.
    println!("Sending GTFRI report (IMEI registration)...");
    writer
        .write_all(GTFRI_LINE.as_bytes())
        .await
        .expect("send failed");

    let mut sack = String::new();
    reader.read_line(&mut sack).await.expect("read SACK failed");
    println!("SACK received: {}", sack.trim_end());
    println!();

    // 2. Send GTHBD heartbeat.
    println!("Sending GTHBD heartbeat...");
    println!("  {}", GTHBD_LINE.trim_end());
    writer
        .write_all(GTHBD_LINE.as_bytes())
        .await
        .expect("send failed");

    let mut hbd_sack = String::new();
    match reader.read_line(&mut hbd_sack).await {
        Ok(0) => eprintln!("Server closed connection without SACK."),
        Ok(_) => println!("SACK received: {}", hbd_sack.trim_end()),
        Err(e) => eprintln!("Error reading SACK: {e}"),
    }

    println!();
    println!("Verify in Valkey:");
    println!("  redis-cli HGETALL devices:{IMEI}");
}
