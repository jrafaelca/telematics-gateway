use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// Paquete cmd=0x44 ExtendedRecords: header(2) + body(240) + CRC(2) = 244 bytes
// Capturado de dispositivo real (IMEI=865262060003118), 2 records.
const PACKET_HEX: &str =
    "00f0000312f385b95f2e440002698a750a001000d6d82cb6f2b2b35c682d688815001f0600090b019901001b14000200000300001c0100202b00ad0101a20100730000cf0000270007001d6ff8001e100000160073001700600074000000c5000000d20000060041004c2679009600011d29005c0000000000720000000000cb0000000000d00000000000698a750a001100d6d82cb6f2b2b35c682d688815001f0600090600ce0000240000230000250000260000c90001002900000200cc000000000482ffffffff04007b0000000000000000007c0000000000000000007d00000000000000000481fffffffffffffffffe20";

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:7700";
    let packet = from_hex(PACKET_HEX);

    println!("Conectando a {}...", addr);
    let mut stream = TcpStream::connect(addr).await.expect("No se pudo conectar");

    println!("Enviando {} bytes...", packet.len());
    stream.write_all(&packet).await.expect("Error enviando paquete");

    // Leer ACK/NACK del servidor (6 bytes: 2 len + 2 body + 2 crc)
    let mut response = [0u8; 16];
    match stream.read(&mut response).await {
        Ok(n) => println!("Respuesta ({} bytes): {:02X?}", n, &response[..n]),
        Err(e) => eprintln!("Error leyendo respuesta: {}", e),
    }
}
