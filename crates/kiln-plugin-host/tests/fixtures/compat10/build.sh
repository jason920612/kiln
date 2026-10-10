#!/bin/sh
# Rebuilds compat10.wasm from the 1.0 WIT.
set -e
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/compat10.wasm compat10.wasm
