//! Device simulator — sends a HeadPack (header 0x01 with IMEI tag).
//!
//! Uses the head packet from the Galileosky protocol specification (page 5).
//!
//! # Usage
//!
//! 1. Start the listener:
//!    ```bash
//!    cargo run -p galileosky-listener
//!    ```
//! 2. Run this example:
//!    ```bash
//!    cargo run -p galileosky-listener --example device_head_packet
//!    ```
//!
//! Expected output: a 3-byte ACK `[02 XX XX]` echoing the packet CRC.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Head packet from the Galileosky specification (page 5), IMEI = 861230043907626.
/// Full frame including 2-byte CRC trailer.
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

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7800";
    let packet = from_hex(HEAD_PACKET_HEX);

    println!("Connecting to {addr}...");
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect — is galileosky-listener running on port 7800?");
    println!("Connected.");

    println!("Sending HeadPack ({} bytes)...", packet.len());
    stream.write_all(&packet).await.expect("send failed");

    // Read the 3-byte ACK: [0x02, crc_lo, crc_hi]
    let mut ack = [0u8; 3];
    match stream.read_exact(&mut ack).await {
        Ok(_) => println!("ACK received: {:02X?}", ack),
        Err(e) => eprintln!("Error reading ACK: {e}"),
    }
}
