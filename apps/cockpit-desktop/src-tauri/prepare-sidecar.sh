#!/usr/bin/env bash
# Build the cockpit-runner and stage it as a Tauri sidecar binary.
#
# Tauri resolves `externalBin` entries by appending the host target triple, so
# the runner is copied to `binaries/cockpit-runner-<triple><ext>`. Run this
# before `npm run tauri:build` (or `tauri:dev`) to package the runner alongside
# the desktop app.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
BIN_DIR="$SCRIPT_DIR/binaries"

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$TRIPLE" ]; then
  echo "could not determine host target triple" >&2
  exit 1
fi

EXT=""
case "$TRIPLE" in
  *windows*) EXT=".exe" ;;
esac

echo "Building cockpit-runner (release) for $TRIPLE"
cargo build --release -p cockpit-runner --manifest-path "$WORKSPACE_ROOT/Cargo.toml"

mkdir -p "$BIN_DIR"
SRC="$WORKSPACE_ROOT/target/release/cockpit-runner$EXT"
DST="$BIN_DIR/cockpit-runner-$TRIPLE$EXT"
cp "$SRC" "$DST"
echo "Staged sidecar: $DST"
