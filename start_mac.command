#!/bin/bash
cd "$(dirname "$0")"
BIN="./target/release/desk-switch"
if [ ! -f "$BIN" ]; then
    echo "Binary not found. Building..."
    cargo build --release
fi
exec $BIN
