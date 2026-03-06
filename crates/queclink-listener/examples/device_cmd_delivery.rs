//! Device simulator — full server→device command delivery flow.
//!
//! Simulates a Queclink GPS device sending a GTFRI report, receiving the
//! server SACK, then receiving and replying to a pending AT+GTRTO command.
//!
//! # Usage
//!
//! 1. Start Valkey and the listener:
//!    ```bash
//!    docker compose up valkey
//!    cargo run -p queclink-listener
//!    ```
//!
//! 2. Insert a pending command in Redis:
//!    ```bash
//!    redis-cli HSET commands:864696060004173 test-uuid-1 \
//!      '{"cmd_text":"AT+GTRTO=gv310lau,3,,,,,","status":"pending"}'
//!    ```
//!
//! 3. Run this example:
//!    ```bash
//!    cargo run -p queclink-listener --example device_cmd_delivery
//!    ```
//!
//! 4. Verify the command was marked delivered:
//!    ```bash
//!    redis-cli HGET commands:864696060004173 test-uuid-1
//!    ```

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

const IMEI: u64 = 864696060004173;
const VERSION: &str = "060100";
const DEVICE_NAME: &str = "queclink";

/// GTFRI line sent by the simulated device.
const GTFRI_LINE: &str = concat!(
    "+RESP:GTFRI,060100,864696060004173,queclink,,0,1,",
    "5,50.5,22.3,250.0,-2.6273,-79.8418,20260305120000,",
    "0730,0002,68C7,5D5A,01,6,20260305120020,0001$\r\n"
);

/// How long to wait for the server to send a command after the SACK.
const WAIT_FOR_CMD: Duration = Duration::from_secs(3);

#[tokio::main]
async fn main() {
    println!("=== device_cmd_delivery ===");
    println!();
    println!("IMEI: {IMEI}");
    println!();
    println!("Before running, insert a pending command in Redis:");
    println!(
        "  redis-cli HSET commands:{IMEI} test-uuid-1 \
        '{{\"cmd_text\":\"AT+GTRTO=gv310lau,3,,,,,,\",\"status\":\"pending\"}}'"
    );
    println!();

    let addr = "127.0.0.1:7950";
    println!("Connecting to {addr}...");
    let stream = TcpStream::connect(addr)
        .await
        .expect("Could not connect — is queclink-listener running?");
    println!("Connected.");
    println!();

    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);

    // 1. Send GTFRI report.
    println!("Sending GTFRI report...");
    writer
        .write_all(GTFRI_LINE.as_bytes())
        .await
        .expect("send failed");
    println!("Sent: {}", GTFRI_LINE.trim_end());

    // 2. Read server SACK.
    let mut sack = String::new();
    reader
        .read_line(&mut sack)
        .await
        .expect("read SACK failed");
    println!("SACK received: {}", sack.trim_end());
    println!();

    // 3. Wait for the server to send a pending AT+GTRTO command.
    println!("Waiting for pending command from server...");
    let mut cmd_line = String::new();
    match timeout(WAIT_FOR_CMD, reader.read_line(&mut cmd_line)).await {
        Err(_) => {
            println!("No command received within timeout.");
            println!("Did you insert the command in Redis before running?");
            println!(
                "  redis-cli HSET commands:{IMEI} test-uuid-1 \
                '{{\"cmd_text\":\"AT+GTRTO=gv310lau,3,,,,,,\",\"status\":\"pending\"}}'"
            );
            return;
        }
        Ok(Err(e)) => {
            eprintln!("Read error: {e}");
            return;
        }
        Ok(Ok(0)) => {
            println!("Server closed connection.");
            return;
        }
        Ok(Ok(_)) => {}
    }

    let cmd_trimmed = cmd_line.trim_end_matches(['\r', '\n']).trim_end_matches('$');
    println!("Command received: {cmd_trimmed}$");

    // 4. Parse the serial number from the AT command (last comma-separated field).
    let serial_str = cmd_trimmed.split(',').last().unwrap_or("0000");
    let serial_num: u16 = u16::from_str_radix(serial_str, 16).unwrap_or(0);
    println!("Serial number: {serial_num:04X}");

    // 5. Send +ACK:GTRTO reply echoing the serial number.
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let send_time = format_datetime(now_ts);

    let ack_line = format!(
        "+ACK:GTRTO,{VERSION},{IMEI:015},{DEVICE_NAME},GPS,{serial_num:04X},{send_time},0002$\r\n"
    );
    println!("Sending ACK: {}", ack_line.trim_end());
    writer
        .write_all(ack_line.as_bytes())
        .await
        .expect("send ACK failed");

    println!();
    println!("Command delivery flow complete.");
    println!("Verify in Redis that status = \"delivered\":");
    println!("  redis-cli HGET commands:{IMEI} test-uuid-1");
}

/// Formats a Unix timestamp (seconds) as `YYYYMMDDHHMMSS`.
fn format_datetime(secs: u64) -> String {
    // Simple UTC conversion sufficient for simulator use.
    let s = secs;
    let sec = s % 60;
    let s = s / 60;
    let min = s % 60;
    let s = s / 60;
    let hour = s % 24;
    let mut days = (s / 24) as u32;

    let mut year = 1970u32;
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }

    const MDAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for &md in &MDAYS {
        let md = if month == 2 && is_leap(year) { md + 1 } else { md };
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days + 1;

    format!("{year:04}{month:02}{day:02}{hour:02}{min:02}{sec:02}")
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
