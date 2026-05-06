#!/usr/bin/env bash
set -euo pipefail

cargo build --target wasm32-unknown-unknown --release -p escrow
