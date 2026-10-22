#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p cindral-app --example import_pushpull_benchmark --features kernel-benchmark
# Each case includes setup; an external deadline catches hangs inside a single update.
for source in imported drawn; do
  CINDRAL_IMPORT_CASE="$source" CINDRAL_ENFORCE_BUDGET=1 timeout 90s "${CARGO_TARGET_DIR:-target}/release/examples/import_pushpull_benchmark"
done
CINDRAL_IMPORT_CASE=imported CINDRAL_IMPORT_GRID=140 CINDRAL_ENFORCE_BUDGET=1 \
  timeout 90s "${CARGO_TARGET_DIR:-target}/release/examples/import_pushpull_benchmark"
if [[ ${CINDRAL_BENCH_GPU:-0} == 1 ]]; then
  cargo build --release -p cindral-app --example import_pushpull_gpu --features kernel-benchmark
  for grid in 64 140; do
    CINDRAL_IMPORT_CASE=imported CINDRAL_IMPORT_GRID="$grid" CINDRAL_ENFORCE_BUDGET=1 \
      timeout 90s "${CARGO_TARGET_DIR:-target}/release/examples/import_pushpull_gpu"
  done
fi
