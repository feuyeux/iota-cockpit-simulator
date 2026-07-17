#!/usr/bin/env bash
set -euo pipefail

readonly DEV_PORT=15342
readonly PORT_RELEASE_ATTEMPTS=20

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}

release_dev_port() {
  local port_pids pid attempt

  port_pids="$(lsof -tiTCP:"$DEV_PORT" -sTCP:LISTEN || true)"
  [[ -z "$port_pids" ]] && return

  echo "Stopping existing development server on port $DEV_PORT"
  while IFS= read -r pid; do
    kill "$pid" 2>/dev/null || true
  done <<< "$port_pids"

  for ((attempt = 1; attempt <= PORT_RELEASE_ATTEMPTS; attempt++)); do
    if ! lsof -tiTCP:"$DEV_PORT" -sTCP:LISTEN >/dev/null 2>&1; then
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
  if [[ -x node_modules/.bin/vite ]]; then
    return 0
  fi

  echo "Frontend dependencies not found, running npm install"
  npm install

  if [[ ! -x node_modules/.bin/vite ]]; then
    echo "npm install completed but vite is still missing from node_modules/.bin" >&2
    exit 1
  fi
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

require_command cargo
require_command lsof
require_command npm

release_dev_port

cd apps/cockpit-desktop

ensure_frontend_deps

echo "Starting Cockpit Desktop on port $DEV_PORT"
npm run tauri:dev
