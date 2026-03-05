//! Device simulator — full server→device command delivery flow.
//!
//! Tests that the server can send a command to the device after ACKing a packet.
//!
//! # Usage
//!
//! 1. Start Valkey and the listener:
//!    ```bash
//!    docker compose up valkey
//!    cargo run -p galileosky-listener
//!    ```
//!
//! 2. Insert a pending command in Redis:
//!    ```bash
//!    redis-cli HSET commands:861230043907626 test-uuid-1 \
//!      '{"cmd_text":"status","status":"pending"}'
//!    ```
//!
//! 3. Run this example:
//!    ```bash
//!    cargo run -p galileosky-listener --example device_cmd_delivery
//!    ```
//!
//! 4. Verify the command was marked delivered:
//!    ```bash
//!    redis-cli HGET commands:861230043907626 test-uuid-1
//!    ```

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

const IMEI: u64 = 861230043907626;

const HEAD_PACKET_HEX: &str = concat!(
    "01", "2000",
    "019A",
    "0218",
    "03", "383631323330303433393037363236",
    "04", "3200",
    "FE", "0600", "010000000000",
    "8F29"
);

/// After each packet ACK the server may immediately deliver pending commands.
/// This timeout determines how long to wait for commands before moving on.
const WAIT_FOR_CMD: Duration = Duration::from_secs(3);

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

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

fn build_main_packet(imei: u64) -> Vec<u8> {
    let imei_str = format!("{imei:015}");
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;

    let mut tags = Vec::new();
    tags.push(0x03u8);
    tags.extend_from_slice(imei_str.as_bytes());
    tags.push(0x20u8);
    tags.extend_from_slice(&now_ts.to_le_bytes());
    // coordinates: correctness=0, satellites=6, lat=48.0, lon=16.0 (Vienna)
    tags.push(0x30u8);
    tags.push(0x06u8);
    tags.extend_from_slice(&(48_000_000i32).to_le_bytes());
    tags.extend_from_slice(&(16_000_000i32).to_le_bytes());
    tags.push(0x33u8);
    tags.extend_from_slice(&200u16.to_le_bytes()); // 20 km/h
    tags.extend_from_slice(&0u16.to_le_bytes());   // 0°
    tags.push(0x34u8);
    tags.extend_from_slice(&180i16.to_le_bytes()); // 180 m

    let tag_len = tags.len() as u16;
    let mut frame = vec![0x01u8, (tag_len & 0xFF) as u8, (tag_len >> 8) as u8];
    frame.extend_from_slice(&tags);
    let crc = crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

/// Reads a Galileosky packet from `stream`.
///
/// Returns the tag section bytes (stripped of header, length and CRC) plus the
/// command number from tag 0xE0 (if present).
async fn read_server_command(stream: &mut TcpStream) -> std::io::Result<(Vec<u8>, Option<u32>)> {
    let mut hdr = [0u8; 1];
    stream.read_exact(&mut hdr).await?;

    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let tag_len = (u16::from_le_bytes(len_buf) & 0x7FFF) as usize;

    let mut tags = vec![0u8; tag_len];
    stream.read_exact(&mut tags).await?;

    let mut crc_buf = [0u8; 2];
    stream.read_exact(&mut crc_buf).await?;
    let crc_recv = u16::from_le_bytes(crc_buf);

    let mut frame = vec![hdr[0], len_buf[0], len_buf[1]];
    frame.extend_from_slice(&tags);
    let crc_calc = crc16_modbus(&frame);

    if crc_recv != crc_calc {
        eprintln!("  [warn] CRC mismatch: recv=0x{crc_recv:04X} calc=0x{crc_calc:04X}");
    }

    // Extract command number from tag 0xE0 in the tag bytes.
    let cmd_number = extract_e0(&tags);
    Ok((tags, cmd_number))
}

/// Extracts the value of tag 0xE0 (4 B LE u32) from a raw tag section.
fn extract_e0(tags: &[u8]) -> Option<u32> {
    let mut i = 0;
    while i < tags.len() {
        let id = tags[i];
        i += 1;
        let size = tag_fixed_size(id)?;
        if id == 0xE0 && i + 4 <= tags.len() {
            return Some(u32::from_le_bytes([tags[i], tags[i+1], tags[i+2], tags[i+3]]));
        }
        i += size;
    }
    None
}

fn tag_fixed_size(id: u8) -> Option<usize> {
    match id {
        0x01 | 0x02 | 0x35 | 0x43 | 0xC4..=0xD2 => Some(1),
        0x04 | 0x10 | 0x34 | 0x40..=0x42 | 0x45 | 0x46
        | 0x50..=0x59 | 0x70..=0x77 | 0xD6..=0xD9 => Some(2),
        0x03 => Some(15),
        0x20 | 0x33 | 0x44 | 0x90 | 0xC0..=0xC3 | 0xD4
        | 0xDB..=0xDF | 0xE0 | 0xE2..=0xE9 => Some(4),
        0x30 => Some(9),
        _ => None, // variable or unknown → stop
    }
}

/// Builds a device reply packet echoing the command number.
fn build_device_reply(imei: u64, cmd_number: u32, reply_text: &str) -> Vec<u8> {
    let imei_str = format!("{imei:015}");
    let text = reply_text.as_bytes();

    let mut tags = Vec::new();
    tags.push(0x03u8);
    tags.extend_from_slice(imei_str.as_bytes());
    tags.push(0xE0u8);
    tags.extend_from_slice(&cmd_number.to_le_bytes());
    tags.push(0xE1u8);
    tags.push(text.len() as u8);
    tags.extend_from_slice(text);

    let tag_len = tags.len() as u16;
    let mut frame = vec![0x01u8, (tag_len & 0xFF) as u8, (tag_len >> 8) as u8];
    frame.extend_from_slice(&tags);
    let crc = crc16_modbus(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

/// Reads the 3-byte ACK from the server and returns it.
async fn read_ack(stream: &mut TcpStream) -> std::io::Result<[u8; 3]> {
    let mut ack = [0u8; 3];
    stream.read_exact(&mut ack).await?;
    Ok(ack)
}

/// Reads and handles all pending server→device commands after an ACK.
///
/// The server may deliver one or more commands immediately after ACKing a
/// packet.  This function drains them, replies to each, and returns the count
/// of commands confirmed.
async fn handle_server_commands(stream: &mut TcpStream, label: &str) -> u32 {
    let mut count = 0u32;
    loop {
        match timeout(WAIT_FOR_CMD, read_server_command(stream)).await {
            Err(_) => {
                // No more commands within the timeout window.
                break;
            }
            Ok(Err(e)) => {
                eprintln!("Error reading command after {label}: {e}");
                break;
            }
            Ok(Ok((tag_bytes, cmd_number_opt))) => {
                let cmd_number = match cmd_number_opt {
                    Some(n) => n,
                    None => {
                        println!("  Received packet without command number (tag 0xE0).");
                        break;
                    }
                };

                println!("  Command received: cmd_number=0x{cmd_number:08X}");
                println!(
                    "  raw tags ({} bytes): {:02X?}",
                    tag_bytes.len(),
                    &tag_bytes[..tag_bytes.len().min(32)]
                );

                let reply = build_device_reply(IMEI, cmd_number, "ok");
                stream.write_all(&reply).await.expect("send reply failed");
                println!("  Device reply sent (cmd_number echoed, text=\"ok\")");
                count += 1;
            }
        }
    }
    count
}

#[tokio::main]
async fn main() {
    println!("=== device_cmd_delivery ===");
    println!();
    println!("IMEI: {IMEI}");
    println!();
    println!("Before running, insert a pending command in Redis:");
    println!("  redis-cli HSET commands:{IMEI} test-uuid-1 \\'{{\"cmd_text\":\"status\",\"status\":\"pending\"}}\\'");
    println!();

    let addr = "127.0.0.1:7800";
    println!("Connecting to {addr}...");
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect — is galileosky-listener running?");
    println!("Connected.");
    println!();

    // 1. Send HeadPack.
    let head = from_hex(HEAD_PACKET_HEX);
    println!("Sending HeadPack ({} bytes)...", head.len());
    stream.write_all(&head).await.expect("send failed");
    let ack = read_ack(&mut stream).await.expect("read HeadPack ACK failed");
    println!("HeadPack ACK: {:02X?}", ack);

    // Handle any commands the server delivers after HeadPack (IMEI just learned).
    println!("Waiting for commands after HeadPack...");
    let after_head = handle_server_commands(&mut stream, "HeadPack").await;
    if after_head > 0 {
        println!("  → {after_head} command(s) delivered after HeadPack.");
    }
    println!();

    // 2. Send MainPack.
    let main = build_main_packet(IMEI);
    println!("Sending MainPack ({} bytes)...", main.len());
    stream.write_all(&main).await.expect("send failed");
    let ack = read_ack(&mut stream).await.expect("read MainPack ACK failed");
    println!("MainPack ACK: {:02X?}", ack);

    // Handle any commands the server delivers after MainPack.
    println!("Waiting for commands after MainPack...");
    let after_main = handle_server_commands(&mut stream, "MainPack").await;
    if after_main > 0 {
        println!("  → {after_main} command(s) delivered after MainPack.");
    }
    println!();

    let total = after_head + after_main;
    if total > 0 {
        println!("Commands confirmed: {total}");
        println!();
        println!("Verify in Redis that status = \"delivered\":");
        println!("  redis-cli HGET commands:{IMEI} test-uuid-1");
    } else {
        println!("No commands received from the server.");
        println!("Did you insert the command in Redis before running the example?");
        println!("  redis-cli HSET commands:{IMEI} test-uuid-1 '{{\"cmd_text\":\"status\",\"status\":\"pending\"}}'");
    }
}
