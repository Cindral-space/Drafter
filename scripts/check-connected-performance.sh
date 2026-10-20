#!/usr/bin/env bash
set -euo pipefail

# Performance regression runner for connected-solid optimization.
# Builds the release connected_benchmark example and runs pushpull/move
# benchmarks for n=60 and n=100 with timeout 10s under CINDRAL_ENFORCE_BUDGET=1.

export CINDRAL_ENFORCE_BUDGET=1

echo "Building release connected_benchmark..."
cargo build --release -p cindral-core --example connected_benchmark --features bench-fixtures

BENCH_BIN="${CARGO_TARGET_DIR:-target}/release/examples/connected_benchmark"

if [ ! -f "$BENCH_BIN" ]; then
    echo "Error: benchmark binary not found at $BENCH_BIN" >&2
    exit 1
fi

echo "Running pushpull n=60..."
timeout 10s "$BENCH_BIN" pushpull 60

echo "Running move n=60..."
timeout 10s "$BENCH_BIN" move 60

echo "Running pushpull n=100..."
timeout 10s "$BENCH_BIN" pushpull 100

echo "Running move n=100..."
timeout 10s "$BENCH_BIN" move 100

echo "All connected performance benchmarks completed successfully."
