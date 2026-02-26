#!/usr/bin/env bash
set -euo pipefail

BINARY="ruptela-listener"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DIST="$WORKSPACE_ROOT/dist"
DATE="$(date +%Y%m%d)"

cd "$WORKSPACE_ROOT"

mkdir -p "$DIST"

for ARCH in amd64 arm64; do
  case "$ARCH" in
    amd64) RUST_TARGET=x86_64-unknown-linux-gnu ;;
    arm64) RUST_TARGET=aarch64-unknown-linux-gnu ;;
  esac

  echo "→ Adding Rust target $RUST_TARGET..."
  rustup target add "$RUST_TARGET"

  echo "→ Compiling $BINARY for $ARCH..."
  cargo zigbuild --release -p "$BINARY" --target "$RUST_TARGET"

  echo "→ Packaging .deb for $ARCH..."
  cargo deb -p "$BINARY" --target "$RUST_TARGET" --no-build --no-strip

  SRC=$(ls "target/debian/${BINARY}_"*"_${ARCH}.deb" | tail -1)
  VERSION=$(basename "$SRC" | sed "s/${BINARY}_//;s/_${ARCH}\.deb//;s/-[0-9]*$//")
  cp "$SRC" "$DIST/${BINARY}_${VERSION}+${DATE}_${ARCH}.deb"
done

echo "✓  Packages:"
ls "$DIST"/${BINARY}_*+${DATE}*.deb
