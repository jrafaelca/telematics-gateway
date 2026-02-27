# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --workspace                          # compile all crates
cargo run -p ruptela-listener                    # start listener on port 7700
cargo test --workspace                           # run all tests
cargo test --workspace <test_name>               # run a single test (e.g. cargo test test_crc16_ack_body)
cargo run -p ruptela-listener --example device_records          # simulate device sending cmd=0x01 Records
cargo run -p ruptela-listener --example device_extended_records  # simulate device sending cmd=0x44 ExtendedRecords
cargo run -p ruptela-listener --example device_cmd_delivery      # simulate full server→device command delivery

docker compose up --build                        # listener + valkey via Docker Compose
```

## Structure (Cargo Workspace)

```
listeners/
├── Cargo.toml                    ← workspace root
├── Dockerfile                    ← multi-stage build for ruptela-listener
├── compose.yaml                  ← ruptela-listener + valkey services
└── crates/
    ├── shared/                   ← shared lib: NormalizedRecord, Publisher
    │   └── src/
    │       ├── lib.rs
    │       ├── normalize.rs      ← NormalizedRecord + to_fields() + stream_key()
    │       └── publisher.rs      ← Publisher (Valkey XADD via ConnectionManager)
    └── ruptela-listener/         ← TCP listener binary
        ├── src/
        │   ├── main.rs           ← CLI args, tracing init, connection semaphore
        │   ├── server.rs         ← handle_connection, send_ack, send_nack
        │   ├── crc.rs            ← CRC-CCITT Kermit (poly 0x8408)
        │   ├── protocol.rs       ← parse_packet, Record, IoElement
        │   └── normalize.rs      ← normalize() converts Record → NormalizedRecord
        ├── tests/                ← integration tests (crc, protocol)
        └── examples/             ← device simulators (x01_test, x44_test)
```

## Architecture

**Packet framing (Ruptela protocol):**
```
[2 bytes: packet_len] [packet_len bytes: body] [2 bytes: CRC16]
```

**Body layout:**
```
[8 bytes: IMEI (u64 BE)] [1 byte: command_id] [variable: payload]
```

**Command handling:**
- `0x01` — Records payload: `[records_left u8][num_records u8][records…]`
- `0x44` — ExtendedRecords payload: same structure, 2-byte IO IDs, wider header
- Unknown commands — stored as raw bytes
- Server ACK: responds with `[command_id + 99, 0x01]` framed with CRC
- Server NACK: responds with `[0x64, 0x00]` framed with CRC

**CRC:** CRC-CCITT Kermit variant — poly `0x8408`, init `0`, computed over the body only (not the 2-byte length header).

**Record structure (per record, variable length):**
- `0x01` header: 23 bytes — timestamp (u32), timestamp_ext (u8), priority (u8), longitude (i32/1e7 → f64), latitude (i32/1e7 → f64), altitude (i16/10 → f32), angle (u16/100 → f32), satellites (u8), speed (u16), hdop (u8/10 → f32), event_id (u8)
- `0x44` header: 25 bytes — same as above but adds record_extension (u8) and widens event_id to u16
- IO elements in four successive groups: 1-byte values, 2-byte values, 4-byte values, 8-byte values. Each group starts with a count byte, then `[id][value]` pairs (id is 1 byte for 0x01, 2 bytes for 0x44).

**Connection flow:** `TcpListener::accept` → acquire semaphore permit (max 10 000) → `tokio::spawn(handle_connection)` — each connection loops reading framed packets until EOF or error.

**Logging:** `tracing` + `tracing-subscriber` (env-filter). Set `RUST_LOG=ruptela_listener=debug` for verbose per-record output.
