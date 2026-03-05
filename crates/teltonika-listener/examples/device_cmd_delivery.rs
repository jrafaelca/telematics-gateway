//! Device simulator — exercises full server→device command delivery via Codec 12.
//!
//! This example simulates the complete flow:
//! 1. IMEI handshake.
//! 2. Send a Codec 8 AVL packet with a valid GPS record.
//! 3. Read the server's 4-byte record-count response.
//! 4. Wait for the server to push a Codec 12 command.
//! 5. Parse the command text and send a Codec 12 response.
//!
//! # Setup
//!
//! First inject a pending command into Valkey:
//! ```bash
//! redis-cli HSET commands:356307042441013 test-uuid \
//!   '{"cmd_text":"getinfo","status":"pending"}'
//! ```
//!
//! Then start the listener and run this example:
//! ```bash
//! docker compose up valkey
//! cargo run -p teltonika-listener
//! cargo run -p teltonika-listener --example device_cmd_delivery
//! ```
//!
//! Verify the command was delivered:
//! ```bash
//! redis-cli HGETALL commands:356307042441013
//! redis-cli HGETALL devices:356307042441013
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

/// Builds a minimal Codec 8 AVL packet with one GPS record.
fn build_avl_packet() -> Vec<u8> {
    let lon_raw = (25.2797_f64 * 10_000_000.0) as i32; // Vilnius longitude
    let lat_raw = (54.6872_f64 * 10_000_000.0) as i32; // Vilnius latitude

    let mut record = Vec::new();
    record.extend_from_slice(&now_ms().to_be_bytes()); // timestamp
    record.push(0x01); // priority
    record.extend_from_slice(&lon_raw.to_be_bytes());
    record.extend_from_slice(&lat_raw.to_be_bytes());
    record.extend_from_slice(&100i16.to_be_bytes()); // altitude
    record.extend_from_slice(&90u16.to_be_bytes());  // angle
    record.push(8u8);                                 // satellites
    record.extend_from_slice(&60u16.to_be_bytes());  // speed
    // IO: no elements.
    record.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    let mut data_field = vec![0x08u8, 0x01u8];
    data_field.extend_from_slice(&record);
    data_field.push(0x01u8);

    let crc = crc16_ibm(&data_field) as u32;
    let dfl = data_field.len() as u32;

    let mut packet = vec![0x00u8, 0x00, 0x00, 0x00];
    packet.extend_from_slice(&dfl.to_be_bytes());
    packet.extend_from_slice(&data_field);
    packet.extend_from_slice(&crc.to_be_bytes());
    packet
}

/// Builds a Codec 12 response packet with the given text.
fn build_codec12_response(response_text: &str) -> Vec<u8> {
    let text_bytes = response_text.as_bytes();
    let resp_len = text_bytes.len() as u32;

    let mut data_field = Vec::new();
    data_field.push(0x0Cu8);                            // codec_id = 12
    data_field.push(0x01u8);                            // quantity 1
    data_field.push(0x06u8);                            // type = response
    data_field.extend_from_slice(&resp_len.to_be_bytes());
    data_field.extend_from_slice(text_bytes);
    data_field.push(0x01u8);                            // quantity 2

    let dfl = data_field.len() as u32;
    let crc = crc16_ibm(&data_field) as u32;

    let mut packet = vec![0x00u8, 0x00, 0x00, 0x00];
    packet.extend_from_slice(&dfl.to_be_bytes());
    packet.extend_from_slice(&data_field);
    packet.extend_from_slice(&crc.to_be_bytes());
    packet
}

/// Reads a Codec 12 command packet from the server and returns the command text.
async fn read_codec12_command(stream: &mut TcpStream) -> Option<String> {
    // Preamble.
    let mut preamble = [0u8; 4];
    stream.read_exact(&mut preamble).await.ok()?;

    // Data field length.
    let mut dfl_buf = [0u8; 4];
    stream.read_exact(&mut dfl_buf).await.ok()?;
    let dfl = u32::from_be_bytes(dfl_buf) as usize;

    // Data field.
    let mut data_field = vec![0u8; dfl];
    stream.read_exact(&mut data_field).await.ok()?;

    // CRC.
    let mut crc_buf = [0u8; 4];
    stream.read_exact(&mut crc_buf).await.ok()?;

    // Validate CRC.
    let crc_recv = u32::from_be_bytes(crc_buf) as u16;
    let crc_calc = crc16_ibm(&data_field);
    if crc_recv != crc_calc {
        println!("  CRC mismatch in command! recv=0x{crc_recv:04X} calc=0x{crc_calc:04X}");
        return None;
    }

    if data_field.len() < 7 || data_field[0] != 0x0C || data_field[2] != 0x05 {
        println!("  Unexpected command format");
        return None;
    }

    let cmd_len = u32::from_be_bytes([
        data_field[3], data_field[4], data_field[5], data_field[6],
    ]) as usize;
    if data_field.len() < 7 + cmd_len {
        return None;
    }
    Some(String::from_utf8_lossy(&data_field[7..7 + cmd_len]).into_owned())
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7900";

    println!("=== device_cmd_delivery ===");
    println!("IMEI: {IMEI}");
    println!();
    println!("Pre-requisite — inject a pending command into Valkey:");
    println!(
        "  redis-cli HSET commands:{IMEI} test-uuid \
         '{{\"cmd_text\":\"getinfo\",\"status\":\"pending\"}}'"
    );
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
    stream.write_all(&handshake).await.expect("send failed");

    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack).await.expect("read failed");
    println!("IMEI response: 0x{:02X} ({})", ack[0], if ack[0] == 1 { "accepted" } else { "rejected" });
    if ack[0] != 1 {
        return;
    }
    println!();

    // 2. Send AVL packet with one GPS record.
    let avl = build_avl_packet();
    println!("Sending AVL packet ({} bytes, lat=54.6872 lon=25.2797)...", avl.len());
    stream.write_all(&avl).await.expect("send failed");

    // 3. Read server's record-count response.
    let mut resp = [0u8; 4];
    stream.read_exact(&mut resp).await.expect("read failed");
    println!("Server accepted: {} records", u32::from_be_bytes(resp));
    println!();

    // 4. Read the Codec 12 command (server delivers after sending record-count response).
    println!("Waiting for Codec 12 command from server...");
    match read_codec12_command(&mut stream).await {
        Some(cmd_text) => {
            println!("Received command: {:?}", cmd_text);

            // 5. Send a Codec 12 response.
            let response = format!("FW ver: 03.27.07 GPS: OK IMEI: {IMEI}");
            let resp_packet = build_codec12_response(&response);
            println!("Sending response ({} bytes): {:?}", resp_packet.len(), response);
            stream.write_all(&resp_packet).await.expect("send failed");
            println!("Response sent.");
        }
        None => {
            println!("No command received (is there a pending command in Valkey?)");
        }
    }

    println!();
    println!("Verify in Valkey:");
    println!("  redis-cli HGETALL commands:{IMEI}");
    println!("  redis-cli HGETALL devices:{IMEI}");
}
