#!/usr/bin/env bash
# Build the cockpit-simulator and stage it as a Tauri sidecar binary.
#
# Tauri resolves `externalBin` entries by appending the host target triple, so
# the simulator is copied to `binaries/cockpit-simulator-<triple><ext>`. Run this
# before `npm run tauri:build` (or `tauri:dev`) to package the simulator alongside
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

mkdir -p "$BIN_DIR"
for NAME in cockpit-simulator cockpit-evaluator; do
  DST="$BIN_DIR/$NAME-$TRIPLE$EXT"
  rm -f "$DST" "$DST.tmp"
done

echo "Building cockpit-simulator and cockpit-evaluator (release) for $TRIPLE"
cargo build --release -p cockpit-simulator -p cockpit-evaluator --features cockpit-simulator/live-acp --manifest-path "$WORKSPACE_ROOT/Cargo.toml"

for NAME in cockpit-simulator cockpit-evaluator; do
  SRC="$WORKSPACE_ROOT/target/release/$NAME$EXT"
  DST="$BIN_DIR/$NAME-$TRIPLE$EXT"
  TMP="$DST.tmp"
  test -f "$SRC"
  cp "$SRC" "$TMP"
  # Tauri copies these files verbatim into the app bundle.
  chmod +x "$TMP"
  mv -f "$TMP" "$DST"
  echo "Staged sidecar: $DST"
done
