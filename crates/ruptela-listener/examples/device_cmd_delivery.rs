//! Simulador de dispositivo con soporte de recepción de comandos server→device.
//!
//! Prueba el flujo completo de entrega de comandos pendientes:
//!
//! 1. Inicia el listener:
//!    ```bash
//!    cargo run -p ruptela-listener
//!    ```
//!
//! 2. Inserta un comando pendiente en Redis (el listener debe estar corriendo):
//!    ```bash
//!    redis-cli HSET commands:13226005504143 test-uuid-1 \
//!      '{"cmd_id":108,"payload":"hola mundo","status":"pending"}'
//!    ```
//!
//! 3. Ejecuta este ejemplo:
//!    ```bash
//!    cargo run -p ruptela-listener --example cmd_delivery_test
//!    ```
//!
//! 4. Verifica que el estado cambió a "delivered":
//!    ```bash
//!    redis-cli HGET commands:13226005504143 test-uuid-1
//!    ```

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

// Mismo paquete cmd=0x01 que usa x01_test.rs (IMEI = 13226005504143)
const PACKET_HEX: &str =
    "033500000C076B5C208F01011E5268CEF20000196E3A3A0AEF3E934F3E2D780000000007000000005268CEFD0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF080000196E3A3A0AEF3E934F3E2D780000000007000000005268CF130000196E3A3A0AEF3E934F3E2D780000000007000000005268CF1E0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF290000196E3A3A0AEF3E934F3E2D780000000007000000005268CF340000196E3A3A0AEF3E934F3E2D780000000007000000005268CF3F0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF4A0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF550000196E3A3A0AEF3E934F3E2D780000000007000000005268CF600000196E3A3A0AEF3E934F3E2D780000000007000000005268CF6B0000196E3A3A0AEF3E934F3E2D780000000007000000005268CF730000196E36630AEF42CE4F6D0BF40400022208000000005268CF7E0000196E36B60AEF42BE4F6D0BF40000000007000000005268CF890000196E36B60AEF42BE4F6D0BF40000000007000000005268CF940000196E36B60AEF42BE4F6D0BF40000000007000000005268CF9F0000196E36B60AEF42BE4F6D0BF40000000007000000005268CFAA0000196E36B60AEF42BE4F6D0BF40000000007000000005268CFB50000196E36B60AEF42BE4F6D0BF40000000007000000005268CFC00000196E36B60AEF42BE4F6D0BF40000000007000000005268CFCB0000196E36B60AEF42BE4F6D0BF40000000007000000005268CFD60000196E36B60AEF42BE4F6D0BF40000000007000000005268CFD70000196E3C710AEF5EFF4F690BF40400011708000000005268CFE20000196E3B980AEF601A4F690BF40000000007000000005268CFED0000196E3B980AEF601A4F690BF40000000007000000005268CFF80000196E3B980AEF601A4F690BF40000000007000000005268D0030000196E3B980AEF601A4F690BF40000000007000000005268D00E0000196E3B980AEF601A4F690BF40000000007000000005268D0190000196E3B980AEF601A4F690BF40000000007000000005268D0240000196E3B980AEF601A4F690BF400000000070000000046E2";

/// IMEI embebido en el paquete de prueba (para usarlo en el redis-cli).
const IMEI: u64 = 13_226_005_504_143;

/// Tiempo máximo de espera para recibir un comando del servidor tras el ACK.
const WAIT_FOR_CMD: Duration = Duration::from_secs(3);

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// CRC-CCITT Kermit (poly 0x8408, init 0) — igual que en crc.rs.
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

/// Lee un paquete Ruptela enmarcado del socket.
///
/// Retorna el body (sin los 2 bytes de longitud ni los 2 de CRC).
/// No valida el CRC — en el simulador lo imprimimos y confiamos.
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

/// Construye el frame de ACK que el dispositivo envía al servidor en respuesta
/// a un comando server→device.
///
/// Formato: [2B len=2][ack_cmd][ack_byte][2B CRC]
/// (sin IMEI — simétrico al ACK que el servidor envía al dispositivo)
fn build_device_ack(ack_cmd: u8, ack_byte: u8) -> Vec<u8> {
    let body = [ack_cmd, ack_byte];
    let crc = crc16(&body);
    let mut frame = vec![0x00, 0x02, ack_cmd, ack_byte];
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

#[tokio::main]
async fn main() {
    println!("=== cmd_delivery_test ===");
    println!();
    println!("IMEI del paquete de prueba: {IMEI}");
    println!();
    println!("Antes de correr este ejemplo, inserta un comando pendiente en Redis:");
    println!(
        "  redis-cli HSET commands:{IMEI} test-uuid-1 \
        '{{\"cmd_id\":108,\"payload\":\"hola mundo\",\"status\":\"pending\"}}'"
    );
    println!();

    let addr = "127.0.0.1:7700";
    println!("Conectando a {addr}...");
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("No se pudo conectar al listener. ¿Está corriendo? (cargo run -p ruptela-listener)");
    println!("Conectado.");
    println!();

    // 1. Enviar paquete de registros (cmd=0x01) como lo haría un dispositivo real.
    let packet = from_hex(PACKET_HEX);
    println!("Enviando paquete cmd=0x01 ({} bytes)...", packet.len());
    stream
        .write_all(&packet)
        .await
        .expect("Error enviando paquete");

    // 2. Leer el ACK del servidor al paquete del dispositivo.
    let ack_body = read_framed(&mut stream)
        .await
        .expect("Error leyendo ACK del servidor");
    println!(
        "Server ACK recibido: {:02X?}  (cmd=0x{:02X})",
        ack_body,
        ack_body.first().copied().unwrap_or(0)
    );
    println!();

    // 3. Leer comandos server→device pendientes (si los hay).
    //    El servidor los envía inmediatamente después del ACK mientras el
    //    dispositivo sigue conectado.
    println!("Esperando comandos del servidor (timeout: {}s)...", WAIT_FOR_CMD.as_secs());
    let mut delivered = 0u32;

    loop {
        let frame_result = timeout(WAIT_FOR_CMD, read_framed(&mut stream)).await;

        match frame_result {
            Err(_) => {
                println!("Sin más comandos del servidor (timeout alcanzado).");
                break;
            }
            Ok(Err(e)) => {
                eprintln!("Error leyendo comando del servidor: {e}");
                break;
            }
            Ok(Ok(body)) => {
                if body.is_empty() {
                    println!("Frame vacío recibido, cerrando.");
                    break;
                }

                let cmd_id = body[0];
                let payload = &body[1..];

                println!(
                    "Comando recibido: cmd=0x{:02X} ({}) payload={:?}",
                    cmd_id,
                    cmd_name(cmd_id),
                    if cmd_id == 0x6C {
                        // Mostrar texto legible para SMS
                        std::str::from_utf8(payload)
                            .map(|s| format!("\"{}\"", s))
                            .unwrap_or_else(|_| format!("{:02X?}", payload))
                    } else {
                        format!("{:02X?}", payload)
                    }
                );

                // Construir y enviar ACK del dispositivo al servidor.
                let (ack_cmd, ack_byte) = device_ack_for(cmd_id);
                let ack_frame = build_device_ack(ack_cmd, ack_byte);
                stream
                    .write_all(&ack_frame)
                    .await
                    .expect("Error enviando device ACK");
                println!(
                    "Device ACK enviado:  cmd=0x{:02X} byte=0x{:02X}",
                    ack_cmd, ack_byte
                );
                println!();
                delivered += 1;
            }
        }
    }

    println!();
    if delivered > 0 {
        println!("Comandos entregados: {delivered}");
        println!();
        println!("Verifica en Redis que el estado es 'delivered':");
        println!("  redis-cli HGET commands:{IMEI} test-uuid-1");
    } else {
        println!("No se recibió ningún comando del servidor.");
        println!("¿Insertaste el comando en Redis antes de correr el ejemplo?");
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

/// Retorna el (ack_cmd, ack_byte) que el dispositivo debe enviar en respuesta.
///
/// | Comando recibido | Respuesta del dispositivo         |
/// |------------------|-----------------------------------|
/// | 0x6C SMS (108)   | cmd=0x07, byte=0x01 (recibido)    |
/// | 0x75 IO  (117)   | cmd=0x11, byte=0x00 (cambiado OK) |
/// | otros            | cmd=mismo+1, byte=0x01            |
fn device_ack_for(cmd_id: u8) -> (u8, u8) {
    match cmd_id {
        0x6C => (0x07, 0x01),
        0x75 => (0x11, 0x00),
        other => (other.wrapping_add(1), 0x01),
    }
}
