#!/usr/bin/env bash
# Cross-platform performance acceptance for the cockpit simulator.
#
# Runs the fixed 1,000-entity / 10,000-events-per-minute workload and writes a
# per-OS report (including peak memory and target triple) so acceptance
# evidence can be collected on Linux, macOS, and Windows (via Git Bash/WSL).
#
# Usage:
#   tools/perf-acceptance.sh [ticks] [out-dir]
set -euo pipefail

TICKS="${1:-120}"
OUT_DIR="${2:-perf-acceptance}"
SCENARIO="scenarios/smoke-in-cockpit.yaml"

mkdir -p "$OUT_DIR"

# Label the artifact by OS so multiple platforms can be aggregated.
OS_LABEL="$(uname -s 2>/dev/null || echo unknown)"
OUT_FILE="$OUT_DIR/bench-${OS_LABEL}.json"

echo "Running perf acceptance: ticks=$TICKS os=$OS_LABEL"
cargo run --release -p cockpit-simulator -- bench "$SCENARIO" \
  --ticks "$TICKS" \
  --active-entities 1000 \
  --events-per-minute 10000 \
  | tee "$OUT_FILE"

echo "Wrote $OUT_FILE"
echo "Note: peak_memory_bytes is captured on Linux (VmHWM). On macOS/Windows"
echo "the report records peak_memory_source explaining why it is absent until a"
echo "platform sampler is wired."
