# listeners

TCP listener for Ruptela GPS/telematics devices. Receives framed binary packets over TCP, validates CRC16, parses GPS records and I/O telemetry, then publishes each record to a Valkey/Redis stream.

## Table of Contents

- [Architecture](#architecture)
- [Protocol](#protocol)
- [Getting Started](#getting-started)
- [Configuration](#configuration)
- [Testing](#testing)
- [Project Structure](#project-structure)

---

## Architecture

```
Device (TCP)
    │
    ▼
TcpListener::accept()
    │
    ├─ tokio::spawn ──▶ handle_connection()
    │                        │
    │                   read 2-byte length
    │                   read body + CRC
    │                   validate CRC16
    │                   parse_packet()
    │                        │
    │                   ┌────┴────────────────┐
    │                   │                     │
    │                Records (0x01)   ExtendedRecords (0x44)
    │                   │                     │
    │                   └────────┬────────────┘
    │                        normalize()
    │                        Publisher::publish()
    │                             │
    │                             ▼
    │                    Valkey XADD devices:records:{shard}
    │
    └─ send ACK / NACK
```

Each connection is handled in its own Tokio task. Publishing is fire-and-forget: each record is spawned as an independent task so a slow Valkey write cannot block the TCP read loop.

---

## Protocol

Ruptela framing (all multi-byte fields big-endian):

```
┌──────────────┬──────────────────────┬──────────┐
│  2 bytes     │  packet_len bytes    │  2 bytes │
│  packet_len  │  body                │  CRC16   │
└──────────────┴──────────────────────┴──────────┘
```

**Body layout:**

```
┌──────────────┬────────────┬─────────────────┐
│  8 bytes     │  1 byte    │  variable       │
│  IMEI (u64)  │ command_id │  payload        │
└──────────────┴────────────┴─────────────────┘
```

**Supported commands:**

| `command_id` | Name              | Payload header              |
|:------------:|-------------------|-----------------------------|
| `0x01`       | Records           | `records_left u8`, `num_records u8`, then records |
| `0x44`       | ExtendedRecords   | Same header; records have a wider IO-ID (2 bytes) and an extra `record_extension` byte |

**Per-record layout (0x01 — 23-byte header):**

```
timestamp(4) timestamp_ext(1) priority(1) longitude(4) latitude(4)
altitude(2) angle(2) satellites(1) speed(2) hdop(1) event_id(1)
[IO groups × 4]
```

**Per-record layout (0x44 — 25-byte header):**

Same as above but with `record_extension(1)` after `timestamp_ext` and `event_id` widened to 2 bytes.

**IO groups** (repeated four times, for 1-, 2-, 4- and 8-byte values):

```
count(1)  [id(1 or 2)  value(N)] × count
```

**CRC:** CRC-CCITT Kermit — polynomial `0x8408` (bit-reversed `0x1021`), init `0x0000`, computed over the body only (the 2-byte length header is excluded).

**Server responses:**

| Response | Body                    |
|----------|-------------------------|
| ACK      | `[command_id + 99, 0x01]` framed with CRC |
| NACK     | `[0x64, 0x00]` framed with CRC            |

---

## Getting Started

### Prerequisites

- Rust stable (install via [rustup](https://rustup.rs))
- Valkey ≥ 7 or Redis ≥ 7 (running locally or reachable)

### Build

**Local development (macOS / native Linux):**

```bash
cargo build --workspace
cargo run -p ruptela-listener

# With explicit flags:
cargo run -p ruptela-listener -- --host 0.0.0.0 --port 5000 --redis-url redis://127.0.0.1:6379
```

**Linux packages for distribution (amd64 + arm64):**

```bash
bash crates/ruptela-listener/build-deb.sh
# → dist/ruptela-listener_<version>_amd64.deb
# → dist/ruptela-listener_<version>_arm64.deb
```

---

## Configuration

All options are CLI flags with sensible defaults. No environment variables or config files required.

| Flag          | Default                    | Description                                         |
|---------------|----------------------------|-----------------------------------------------------|
| `--host`      | `127.0.0.1`                | Address to bind the TCP listener                    |
| `--port`      | `5000`                     | Port to bind the TCP listener                       |
| `--redis-url` | `redis://127.0.0.1:6379`   | Valkey/Redis connection URL                         |
| `--shards`    | `8`                        | Number of stream shards (records are distributed via `IMEI % shards`) |

Stream keys follow the pattern `devices:records:{shard}`.

---

## Testing

```bash
# Run all tests
cargo test --workspace

# Only CRC tests
cargo test --workspace --test crc

# Only protocol parser tests
cargo test --workspace --test protocol

# Verify no inline unit tests remain in lib
cargo test --workspace --lib
```

### Device simulators

Two example binaries simulate real device connections. Start the listener first, then run either simulator:

```bash
# Terminal 1
cargo run -p ruptela-listener

# Terminal 2 — sends a cmd=0x01 Records packet (825 bytes, 30 records)
cargo run -p ruptela-listener --example x01_test

# Terminal 2 — sends a cmd=0x44 ExtendedRecords packet (244 bytes, 2 records with 2-byte IO IDs)
cargo run -p ruptela-listener --example x44_test
```

---

## Project Structure

```
listeners/
├── crates/ruptela-listener/
│   └── build-deb.sh                # produces dist/*.deb for amd64 and arm64
├── crates/
│   ├── shared/                     # NormalizedRecord, Publisher
│   │   └── src/
│   │       ├── normalize.rs        # NormalizedRecord + to_fields() + stream_key()
│   │       └── publisher.rs        # Publisher (Valkey XADD via ConnectionManager)
│   └── ruptela-listener/           # TCP listener binary
│       ├── README.md               # deployment guide for the client
│       ├── src/
│       │   ├── main.rs             # CLI args, tracing init, connection semaphore
│       │   ├── server.rs           # handle_connection, send_ack, send_nack
│       │   ├── protocol.rs         # parse_packet, Record, IoElement
│       │   ├── crc.rs              # CRC-CCITT Kermit (poly 0x8408)
│       │   └── normalize.rs        # normalize() converts Record → NormalizedRecord
│       ├── tests/
│       │   ├── common.rs           # shared hex helper and packet fixtures
│       │   ├── crc.rs              # CRC tests
│       │   └── protocol.rs         # parser tests
│       └── examples/
│           ├── x01_test.rs         # simulator: cmd=0x01 Records
│           └── x44_test.rs         # simulator: cmd=0x44 ExtendedRecords
```
