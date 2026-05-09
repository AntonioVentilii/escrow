#!/usr/bin/env bash
set -euo pipefail

cargo build --locked --target wasm32-unknown-unknown --release -p escrow

gzip -c target/wasm32-unknown-unknown/release/escrow.wasm >target/wasm32-unknown-unknown/release/escrow.wasm.gz
