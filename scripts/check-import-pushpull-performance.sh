#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p cindral-app --example import_pushpull_benchmark
# Each case includes setup; an external deadline catches hangs inside a single update.
for source in imported drawn; do
  CINDRAL_IMPORT_CASE="$source" CINDRAL_ENFORCE_BUDGET=1 timeout 90s "${CARGO_TARGET_DIR:-target}/release/examples/import_pushpull_benchmark"
done
