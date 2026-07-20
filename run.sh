#!/usr/bin/env bash
set -euo pipefail

readonly DEV_PORT=15342
readonly PORT_RELEASE_ATTEMPTS=20
CLEAN=false

usage() {
  echo "Usage: $0 [--clean]" >&2
}

while (($# > 0)); do
  case "$1" in
    --clean)
      CLEAN=true
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
  shift
done

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}

# git-bash on Windows does not ship `lsof`, so fall back to `netstat -ano` +
# `taskkill` when the Unix tool is unavailable. The check happens once per
# run, not per attempt, to keep the polling loop cheap.
if command -v lsof >/dev/null 2>&1; then
  port_pids_listening() {
    lsof -tiTCP:"$DEV_PORT" -sTCP:LISTEN 2>/dev/null || true
  }
else
  port_pids_listening() {
    # netstat prints lines like "  TCP    0.0.0.0:15342    0.0.0.0:0    LISTENING    1234"
    netstat -ano -p TCP 2>/dev/null \
      | awk -v port=":$DEV_PORT" '$0 ~ port && $0 ~ /LISTENING/ { print $NF }' \
      | sort -u
  }
fi

release_dev_port() {
  local port_pids pid attempt

  port_pids="$(port_pids_listening)"
  [[ -z "$port_pids" ]] && return

  echo "Stopping existing development server on port $DEV_PORT"
  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    if command -v taskkill >/dev/null 2>&1; then
      # Invoke through cmd.exe: bash on Windows shells `taskkill` and MSYS
      # path-conversion strips a single `/F` to `/F` (fine) but if the
      # argument starts with `/` MSYS also tries to convert it, leaving
      # `taskkill` with a literal `//F` it rejects. Going through `cmd /c`
      # sidesteps that and works whether or not MSYS is involved.
      cmd //c "taskkill /F /PID $pid" >/dev/null 2>&1 || true
    else
      kill "$pid" 2>/dev/null || true
    fi
  done <<< "$port_pids"

  for ((attempt = 1; attempt <= PORT_RELEASE_ATTEMPTS; attempt++)); do
    if [[ -z "$(port_pids_listening)" ]]; then
      return
    fi
    sleep 0.1
  done

  echo "Port $DEV_PORT is still in use; stop its listener and try again." >&2
  exit 1
}

ensure_frontend_deps() {
  # A one-click dev entry point cannot assume `npm install` was already run.
  # `node_modules/.bin/vite` is the concrete artifact `tauri dev`'s
  # beforeDevCommand needs; its absence is what produces a confusing
  # "vite: command not found" failure deep inside the tauri process, so treat
  # it as the install signal rather than only checking for `node_modules`.
  # On Windows, npm writes `vite.cmd` (not the bare `vite`) inside .bin, so
  # accept either form.
  if [[ -x node_modules/.bin/vite ]] || [[ -x node_modules/.bin/vite.cmd ]]; then
    return 0
  fi

  echo "Frontend dependencies not found, running npm install"
  npm install

  if [[ ! -x node_modules/.bin/vite ]] && [[ ! -x node_modules/.bin/vite.cmd ]]; then
    echo "npm install completed but vite is still missing from node_modules/.bin" >&2
    exit 1
  fi
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

require_command cargo
require_command npm
# `lsof` is preferred for releasing $DEV_PORT but is missing on git-bash;
# `release_dev_port` falls back to `netstat -ano` + `taskkill` when absent.

if [[ "$CLEAN" == true ]]; then
  echo "Cleaning Rust workspace (requested with --clean)"
  cargo clean
fi

release_dev_port

cd apps/cockpit-desktop

ensure_frontend_deps

echo "Starting Cockpit Desktop on port $DEV_PORT"
npm run tauri:dev
