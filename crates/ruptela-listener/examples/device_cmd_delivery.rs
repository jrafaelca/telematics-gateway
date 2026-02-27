//! Device simulator with support for receiving server→device commands.
//!
//! Tests the full command delivery flow:
//!
//! 1. Start the listener:
//!    ```bash
//!    cargo run -p ruptela-listener
//!    ```
//!
//! 2. Insert a pending command in Redis (the listener must be running):
//!    ```bash
//!    redis-cli HSET commands:13226005504143 test-uuid-1 \
//!      '{"cmd_id":108,"payload":"hello world","status":"pending"}'
//!    ```
//!
//! 3. Run this example:
//!    ```bash
//!    cargo run -p ruptela-listener --example device_cmd_delivery
//!    ```
//!
//! 4. Verify that the status changed to "delivered":
//!    ```bash
//!    redis-cli HGET commands:13226005504143 test-uuid-1
//!    ```

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

// Same cmd=0x01 packet used in device_records.rs (IMEI = 13226005504143)
const PACKET_HEX: &str =
    "033500000C076B5C208F01011E5268CEF20000196E3A3A0AEF3E934F3E2D780000000007000000005268CEFD0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF080000196E3A3A0AEF3E934F3E2D780000000007000000005268CF130000196E3A3A0AEF3E934F3E2D780000000007000000005268CF1E0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF290000196E3A3A0AEF3E934F3E2D780000000007000000005268CF340000196E3A3A0AEF3E934F3E2D780000000007000000005268CF3F0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF4A0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF550000196E3A3A0AEF3E934F3E2D780000000007000000005268CF600000196E3A3A0AEF3E934F3E2D780000000007000000005268CF6B0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF730000196E36630AEF42CE4F6D0BF40400022208000000005268CF7E0000196E36B60AEF42BE4F6D0BF40000000007000000005268CF890000196E36B60AEF42BE4F6D0BF40000000007000000005268CF940000196E36B60AEF42BE4F6D0BF40000000007000000005268CF9F0000196E36B60AEF42BE4F6D0BF40000000007000000005268CFAA0000196E36B60AEF42BE4F6D0BF40000000007000000005268CFB50000196E36B60AEF42BE4F6D0BF40000000007000000005268CFC00000196E36B60AEF42BE4F6D0BF40000000007000000005268CFCB0000196E36B60AEF42BE4F6D0BF40000000007000000005268CFD60000196E36B60AEF42BE4F6D0BF40000000007000000005268CFD70000196E3C710AEF5EFF4F690BF40400011708000000005268CFE20000196E3B980AEF601A4F690BF40000000007000000005268CFED0000196E3B980AEF601A4F690BF40000000007000000005268CFF80000196E3B980AEF601A4F690BF40000000007000000005268D0030000196E3B980AEF601A4F690BF40000000007000000005268D00E0000196E3B980AEF601A4F690BF40000000007000000005268D0190000196E3B980AEF601A4F690BF40000000007000000005268D0240000196E3B980AEF601A4F690BF400000000070000000046E2";

/// IMEI embedded in the test packet (for use with redis-cli).
const IMEI: u64 = 13_226_005_504_143;

/// Maximum wait time to receive a server command after the ACK.
const WAIT_FOR_CMD: Duration = Duration::from_secs(3);

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// CRC-CCITT Kermit (poly 0x8408, init 0) — same as in crc.rs.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        let mut b = byte;
        for _ in 0..8 {
            let mix = (crc ^ b as u16) & 0x01;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8408;
            }
            b >>= 1;
        }
    }
    crc
}

/// Reads a framed Ruptela packet from the socket.
///
/// Returns the body (without the 2-byte length header or 2-byte CRC).
/// Does not validate the CRC — the simulator prints it and trusts it.
async fn read_framed(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let packet_len = u16::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; packet_len + 2]; // body + CRC
    stream.read_exact(&mut buf).await?;

    let body = buf[..packet_len].to_vec();
    let crc_recv = u16::from_be_bytes([buf[packet_len], buf[packet_len + 1]]);
    let crc_calc = crc16(&body);
    if crc_recv != crc_calc {
        eprintln!(
            "  [warn] CRC mismatch: recv=0x{:04X} calc=0x{:04X}",
            crc_recv, crc_calc
        );
    }
    Ok(body)
}

/// Builds the ACK frame that the device sends to the server in response
/// to a server→device command.
///
/// Format: [2B len=2][ack_cmd][ack_byte][2B CRC]
/// (no IMEI — symmetric to the ACK the server sends to the device)
fn build_device_ack(ack_cmd: u8, ack_byte: u8) -> Vec<u8> {
    let body = [ack_cmd, ack_byte];
    let crc = crc16(&body);
    let mut frame = vec![0x00, 0x02, ack_cmd, ack_byte];
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

#[tokio::main]
async fn main() {
    println!("=== device_cmd_delivery ===");
    println!();
    println!("Test packet IMEI: {IMEI}");
    println!();
    println!("Before running this example, insert a pending command in Redis:");
    println!(
        "  redis-cli HSET commands:{IMEI} test-uuid-1 \
        '{{\"cmd_id\":108,\"payload\":\"hello world\",\"status\":\"pending\"}}'"
    );
    println!();

    let addr = "127.0.0.1:7700";
    println!("Connecting to {addr}...");
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect to the listener. Is it running? (cargo run -p ruptela-listener)");
    println!("Connected.");
    println!();

    // 1. Send a records packet (cmd=0x01) as a real device would.
    let packet = from_hex(PACKET_HEX);
    println!("Sending cmd=0x01 packet ({} bytes)...", packet.len());
    stream
        .write_all(&packet)
        .await
        .expect("Error sending packet");

    // 2. Read the server ACK for the device packet.
    let ack_body = read_framed(&mut stream)
        .await
        .expect("Error reading server ACK");
    println!(
        "Server ACK received: {:02X?}  (cmd=0x{:02X})",
        ack_body,
        ack_body.first().copied().unwrap_or(0)
    );
    println!();

    // 3. Read pending server→device commands (if any).
    //    The server sends them immediately after the ACK while the
    //    device is still connected.
    println!("Waiting for server commands (timeout: {}s)...", WAIT_FOR_CMD.as_secs());
    let mut delivered = 0u32;

    loop {
        let frame_result = timeout(WAIT_FOR_CMD, read_framed(&mut stream)).await;

        match frame_result {
            Err(_) => {
                println!("No more server commands (timeout reached).");
                break;
            }
            Ok(Err(e)) => {
                eprintln!("Error reading server command: {e}");
                break;
            }
            Ok(Ok(body)) => {
                if body.is_empty() {
                    println!("Empty frame received, closing.");
                    break;
                }

                let cmd_id = body[0];
                let payload = &body[1..];

                println!(
                    "Command received: cmd=0x{:02X} ({}) payload={:?}",
                    cmd_id,
                    cmd_name(cmd_id),
                    if cmd_id == 0x6C {
                        // Show readable text for SMS
                        std::str::from_utf8(payload)
                            .map(|s| format!("\"{}\"", s))
                            .unwrap_or_else(|_| format!("{:02X?}", payload))
                    } else {
                        format!("{:02X?}", payload)
                    }
                );

                // Build and send the device ACK to the server.
                let (ack_cmd, ack_byte) = device_ack_for(cmd_id);
                let ack_frame = build_device_ack(ack_cmd, ack_byte);
                stream
                    .write_all(&ack_frame)
                    .await
                    .expect("Error sending device ACK");
                println!(
                    "Device ACK sent:     cmd=0x{:02X} byte=0x{:02X}",
                    ack_cmd, ack_byte
                );
                println!();
                delivered += 1;
            }
        }
    }

    println!();
    if delivered > 0 {
        println!("Commands delivered: {delivered}");
        println!();
        println!("Verify in Redis that the status is 'delivered':");
        println!("  redis-cli HGET commands:{IMEI} test-uuid-1");
    } else {
        println!("No commands were received from the server.");
        println!("Did you insert the command in Redis before running the example?");
        println!("  redis-cli HGET commands:{IMEI} test-uuid-1");
    }
}

fn cmd_name(cmd_id: u8) -> &'static str {
    match cmd_id {
        0x6C => "SMS via GPRS",
        0x75 => "Set IO Value",
        _ => "unknown",
    }
}

/// Returns the (ack_cmd, ack_byte) that the device should send in response.
///
/// | Received command | Device response                   |
/// |------------------|-----------------------------------|
/// | 0x6C SMS (108)   | cmd=0x07, byte=0x01 (received)    |
/// | 0x75 IO  (117)   | cmd=0x11, byte=0x00 (changed OK)  |
/// | other            | cmd=same+1, byte=0x01             |
fn device_ack_for(cmd_id: u8) -> (u8, u8) {
    match cmd_id {
        0x6C => (0x07, 0x01),
        0x75 => (0x11, 0x00),
        other => (other.wrapping_add(1), 0x01),
    }
}
