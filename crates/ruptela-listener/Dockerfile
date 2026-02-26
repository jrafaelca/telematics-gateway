FROM --platform=$BUILDPLATFORM rust:1.85-slim AS builder
ARG TARGETARCH=amd64
WORKDIR /app

RUN apt-get update && apt-get install -y \
    gcc-x86-64-linux-gnu \
    gcc-aarch64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

COPY . .

RUN set -e; \
    case "$TARGETARCH" in \
      amd64) RUST_TARGET=x86_64-unknown-linux-gnu; \
             LINKER_VAR=CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER; \
             LINKER_BIN=x86_64-linux-gnu-gcc ;; \
      arm64) RUST_TARGET=aarch64-unknown-linux-gnu; \
             LINKER_VAR=CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER; \
             LINKER_BIN=aarch64-linux-gnu-gcc ;; \
      *) echo "Unsupported TARGETARCH=$TARGETARCH" && exit 1 ;; \
    esac; \
    env "${LINKER_VAR}=${LINKER_BIN}" \
    cargo build --release -p ruptela-listener --target "$RUST_TARGET"; \
    cp "target/$RUST_TARGET/release/ruptela-listener" /binary

FROM debian:bookworm-slim
RUN addgroup --system ruptela && adduser --system --ingroup ruptela ruptela
COPY --from=builder /binary /usr/ruptela/listener
EXPOSE 5000
USER ruptela
ENTRYPOINT ["/usr/ruptela/listener"]
