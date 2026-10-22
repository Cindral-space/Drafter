#!/usr/bin/env bash
# Private fixture is optional, and is never copied into the checkout.
set -euo pipefail
cd "$(dirname "$0")/.."
seed="${CINDRAL_REAL_FILE:-SKP/mybedroom.skp}"
if [[ ! -f "$seed" ]]; then
    echo 'SKIP: mybedroom.skp absent; set CINDRAL_REAL_FILE to its private path.'
    exit 0
fi
cargo build --release -p cindral-app --features kernel-benchmark --example real_file_benchmark
CINDRAL_PROFILE=1 CINDRAL_ENFORCE_BUDGET=1 CINDRAL_BENCH_GPU="${CINDRAL_BENCH_GPU:-1}" \
    timeout 120s "${CARGO_TARGET_DIR:-target}/release/examples/real_file_benchmark" "$seed"
