#!/usr/bin/env bash
set -euo pipefail

readonly DEV_PORT=15342
readonly PORT_RELEASE_ATTEMPTS=20
CLEAN=false

usage() {
  echo "Usage: $0 [--clean]" >&2
  echo >&2
  echo "Cockpit Desktop is a Tauri 2 app whose Webview loads the React frontend" >&2
  echo "from a running Vite dev server (default http://127.0.0.1:15342). Tauri's" >&2
  echo "beforeDevCommand in apps/cockpit-desktop/src-tauri/tauri.conf.json is what" >&2
  echo "starts that server, so this script always launches the app via" >&2
  echo "'npm run tauri:dev'." >&2
  echo >&2
  echo "Do NOT run the compiled binary directly (e.g." >&2
  echo "  cargo run --bin cockpit-desktop" >&2
  echo "  target/debug/cockpit-desktop" >&2
  echo "). Skipping beforeDevCommand leaves the Webview connecting to a port" >&2
  echo "nothing is listening on, so the window appears blank." >&2
  echo >&2
  echo "To produce a self-contained bundle that does not need Vite, run" >&2
  echo "  npm run tauri:build" >&2
  echo "from apps/cockpit-desktop instead." >&2
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

# If a previous Tauri build artefact is present but the Vite dev server is not
# running, Tauri would still try to load the devUrl and produce a blank window.
# Surface that condition loudly instead of waiting for the user to discover it
# by staring at the screen.
if [[ ! -d node_modules/.vite ]] \
    && [[ ! -f node_modules/vite/bin/vite.js ]] \
    && [[ ! -d ../dist ]]; then
  echo "No built frontend (apps/cockpit-desktop/dist) and no Vite dev cache found." >&2
  echo "Proceeding will start Vite for the first time, which can take a while." >&2
fi

if [[ -d ../dist ]] && [[ -z "$(port_pids_listening)" ]]; then
  # A built frontend exists but Vite is not running: launching tauri:dev will
  # rebuild the dist on the fly. That is fine, but make it visible so the
  # blank window during rebuild is not mistaken for a regression.
  echo "Detected existing apps/cockpit-desktop/dist without a running Vite server." >&2
  echo "tauri:dev will reuse or rebuild that bundle before the Webview connects." >&2
fi

echo "Starting Cockpit Desktop on port $DEV_PORT"
npm run tauri:dev
