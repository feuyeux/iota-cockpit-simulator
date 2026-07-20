#!/usr/bin/env bash
set -Eeuo pipefail

# Attach to an already running Cockpit Desktop (or one of its child processes)
# and collect CPU or allocation-stack profiles on Linux and macOS.

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

PROFILE_TYPE="${PROFILE_TYPE:-all}"
DURATION="${DURATION:-45}"
PID="${PID:-}"
PROCESS_PATTERN="${PROCESS_PATTERN:-}"
OUTPUT_DIR="${OUTPUT_DIR:-$ROOT_DIR/profile-results}"
AUTO_UPDATE=1
ENABLE_DEVELOPER_TOOLS=0
PID_WAS_EXPLICIT=0
PROCESS_PATTERN_WAS_EXPLICIT=0
ORIGINAL_UID="$(id -u)"
ORIGINAL_GID="$(id -g)"

log() { printf '[profile] %s\n' "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }
on_error() { log "Command failed at line $1. Partial files are kept in: $OUTPUT_DIR"; }
trap 'on_error "$LINENO"' ERR

usage() {
  cat <<'EOF'
Usage: tools/profile-desktop.sh [cpu|memory] [options]

Omit cpu/memory to collect both, in order: CPU then memory.

Options:
  --pid PID             Override discovery and attach to an exact process.
  --process REGEX       Override automatic Desktop process discovery.
  --duration SECONDS    Sampling duration (default: 45).
  --output DIR          Result directory (default: profile-results).
  --no-update           Do not install or upgrade profiling tools.
  --enable-developer-tools
                      Force the macOS Developer Tools access prompt before
                      profiling starts.
  -h, --help            Show this help.

Examples:
  tools/profile-desktop.sh
  tools/profile-desktop.sh cpu
  tools/profile-desktop.sh memory --duration 60
  tools/profile-desktop.sh memory --enable-developer-tools
EOF
}

while (($#)); do
  case "$1" in
    cpu|memory) PROFILE_TYPE="$1"; shift ;;
    --pid) PID="${2:?missing PID}"; PID_WAS_EXPLICIT=1; shift 2 ;;
    --process) PROCESS_PATTERN="${2:?missing regex}"; PROCESS_PATTERN_WAS_EXPLICIT=1; shift 2 ;;
    --duration) DURATION="${2:?missing seconds}"; shift 2 ;;
    --output) OUTPUT_DIR="${2:?missing directory}"; shift 2 ;;
    --no-update) AUTO_UPDATE=0; shift ;;
    --enable-developer-tools) ENABLE_DEVELOPER_TOOLS=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[[ "$PROFILE_TYPE" == cpu || "$PROFILE_TYPE" == memory || "$PROFILE_TYPE" == all ]] || die "Type must be cpu or memory."
[[ "$DURATION" =~ ^[1-9][0-9]*$ ]] || die "Duration must be a positive integer."
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd -- "$OUTPUT_DIR" && pwd)"

run_privileged() {
  if [[ ${EUID:-$(id -u)} -eq 0 ]]; then "$@"; else command -v sudo >/dev/null || die "sudo is required for this operation."; sudo "$@"; fi
}

install_linux_packages() {
  local packages=("$@")
  if command -v apt-get >/dev/null; then
    run_privileged apt-get update
    run_privileged apt-get install -y --only-upgrade "${packages[@]}" 2>/dev/null || true
    run_privileged apt-get install -y "${packages[@]}"
  elif command -v dnf >/dev/null; then
    run_privileged dnf install -y --refresh "${packages[@]}"
  elif command -v zypper >/dev/null; then
    run_privileged zypper --non-interactive refresh
    run_privileged zypper --non-interactive install --force-resolution "${packages[@]}"
  elif command -v pacman >/dev/null; then
    run_privileged pacman -Syu --needed --noconfirm "${packages[@]}"
  else
    die "Unsupported Linux package manager. Install ${packages[*]} manually, then use --no-update."
  fi
}

ensure_linux_tools() {
  ((AUTO_UPDATE)) || return 0
  case "$PROFILE_TYPE" in
    cpu)
      if command -v apt-get >/dev/null; then install_linux_packages linux-tools-common "linux-tools-$(uname -r)" git perl
      else install_linux_packages perf git perl
      fi
      ;;
    memory) install_linux_packages heaptrack ;;
  esac
}

ensure_flamegraph() {
  local destination="${XDG_CACHE_HOME:-$HOME/.cache}/cockpit-profiler/FlameGraph"
  if [[ -d "$destination/.git" ]]; then
    ((AUTO_UPDATE)) && git -C "$destination" pull --ff-only
  else
    ((AUTO_UPDATE)) || die "FlameGraph is missing. Re-run without --no-update."
    command -v git >/dev/null || die "git is required to install FlameGraph."
    mkdir -p "$(dirname -- "$destination")"
    git clone --depth 1 https://github.com/brendangregg/FlameGraph.git "$destination"
  fi
  FLAMEGRAPH_DIR="$destination"
}

find_pid() {
  if [[ -n "$PID" ]]; then
    kill -0 "$PID" 2>/dev/null || die "PID $PID is not running or is not accessible."
    return
  fi
  local matches pattern
  if [[ -n "$PROCESS_PATTERN" ]]; then
    pattern="$PROCESS_PATTERN"
    if [[ "$(uname -s)" == Darwin ]]; then
      matches="$(pgrep -ifl "$pattern" || true)"
    else
      matches="$(pgrep -af "$pattern" || true)"
    fi
  else
    # Match executable paths only. Development commands contain the project
    # directory, so searching their full command lines selects esbuild/Vite.
    if [[ "$(uname -s)" == Darwin ]]; then
      matches="$(ps -axo pid=,comm= | awk 'substr($0, length($0) - length("/cockpit-desktop") + 1) == "/cockpit-desktop" || (index($0, "Cockpit Simulation.app/Contents/MacOS/") && index($0, "Cockpit Simulation")) {print $1}')"
    else
      matches="$(ps -eo pid=,comm= | awk 'substr($0, length($0) - length("/cockpit-desktop") + 1) == "/cockpit-desktop" {print $1}')"
    fi
  fi
  if [[ -z "$matches" && -z "$PROCESS_PATTERN" ]]; then
    # Fall back to the sidecar only when the Tauri host is not present.
    if [[ "$(uname -s)" == Darwin ]]; then
      matches="$(ps -axo pid=,comm= | awk 'substr($0, length($0) - length("/cockpit-simulator") + 1) == "/cockpit-simulator" {print $1}')"
    else
      matches="$(ps -eo pid=,comm= | awk 'substr($0, length($0) - length("/cockpit-simulator") + 1) == "/cockpit-simulator" {print $1}')"
    fi
  fi
  [[ -n "$matches" ]] || die "Cockpit Desktop is not running. Start it first with ./run.sh or npm run tauri:dev."

  # pgrep returns PIDs in ascending order on supported platforms; the final
  # entry is the newest matching process and avoids stale development hosts.
  PID="$(printf '%s\n' "$matches" | tail -1 | awk '{print $1}')"
  [[ "$PID" =~ ^[0-9]+$ ]] || die "Could not determine a valid Desktop PID."
  log "Automatically selected Desktop PID $PID."
}

ensure_target_running() {
  kill -0 "$PID" 2>/dev/null || die "Desktop PID $PID exited before the $PROFILE_TYPE profile started. Restart the Desktop and rerun this command."
}

fix_output_ownership() {
  local path="$1"
  [[ -e "$path" ]] || return 0
  if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    chown -R "$ORIGINAL_UID:$ORIGINAL_GID" "$path"
  else
    sudo chown -R "$ORIGINAL_UID:$ORIGINAL_GID" "$path"
  fi
}

current_pid_command() {
  ps -ww -p "$PID" -o command= | sed 's/[[:space:]]\+$//'
}

ensure_macos_tools() {
  if ! command -v xcrun >/dev/null; then
    if ((AUTO_UPDATE)) && command -v xcode-select >/dev/null; then
      xcode-select --install 2>/dev/null || true
    fi
    die "Xcode profiling tools are unavailable. Complete the Xcode Command Line Tools installation, then retry."
  fi
  xcrun --find xctrace >/dev/null || die "xctrace is missing; update Xcode or Command Line Tools."
}

ensure_macos_instruments_access() {
  local developer_tools_security="/usr/sbin/DevToolsSecurity"
  [[ -x "$developer_tools_security" ]] || return 0

  local status
  status="$(LC_ALL=C "$developer_tools_security" -status 2>&1 || true)"
  [[ "$status" == *disabled* ]] || return 0

  log "Enabling macOS Developer Tools access for Instruments (one-time system setting)."
  if ((ENABLE_DEVELOPER_TOOLS)); then
    log "Developer Tools access is disabled; prompting now."
  fi
  run_privileged "$developer_tools_security" -enable
  status="$(LC_ALL=C "$developer_tools_security" -status 2>&1 || true)"
  [[ "$status" != *disabled* ]] || die "Developer Tools access is still disabled after the authorization attempt."
}

launch_macos_debug_bundle() {
  local desktop_dir="$ROOT_DIR/apps/cockpit-desktop"
  local bundle_dir bundle_executable

  log "Building macOS debug bundle for attachable profiling."
  (cd "$desktop_dir" && npm run tauri:build -- --debug)

  bundle_dir="$(find "$desktop_dir/src-tauri/target/debug/bundle" -type d -name '*.app' | sort | tail -1)"
  [[ -n "$bundle_dir" ]] || die "Could not find the macOS debug bundle after building."

  bundle_executable="$bundle_dir/Contents/MacOS/$(basename "${bundle_dir%.app}")"
  [[ -x "$bundle_executable" ]] || die "Debug bundle executable is missing: $bundle_executable"

  log "Launching debug bundle for Instruments attach: $bundle_executable"
  "$bundle_executable" >/dev/null 2>&1 &
  PID="$!"
  sleep 2
  kill -0 "$PID" 2>/dev/null || die "Debug bundle exited immediately after launch."
}

maybe_switch_macos_memory_target() {
  [[ "$(uname -s)" == Darwin ]] || return 0
  [[ "$PROFILE_TYPE" == memory || "$PROFILE_TYPE" == all ]] || return 0
  ((PID_WAS_EXPLICIT)) && return 0
  ((PROCESS_PATTERN_WAS_EXPLICIT)) && return 0

  local command_line
  command_line="$(current_pid_command)"
  if [[ "$command_line" == *"/target/debug/cockpit-desktop"* ]]; then
    log "Current Desktop build is the unbundled debug binary; switching to the signed debug bundle for memory profiling."
    launch_macos_debug_bundle
  fi
}

includes_memory_profile() {
  local profile
  for profile in "${profile_types[@]}"; do
    [[ "$profile" == memory ]] && return 0
  done
  return 1
}

timestamp="$(date +%Y%m%d-%H%M%S)"
find_pid
if [[ "$PROFILE_TYPE" == all ]]; then
  profile_types=(cpu memory)
  log "Collecting CPU first, then memory, for Desktop PID $PID."
else
  profile_types=("$PROFILE_TYPE")
fi

# Validate the one-time Instruments authorization before an `all` run spends
# time collecting CPU data that would otherwise be followed by a doomed attach.
if [[ "$(uname -s)" == Darwin ]] && includes_memory_profile; then
  ensure_macos_tools
  ensure_macos_instruments_access
fi

for PROFILE_TYPE in "${profile_types[@]}"; do
  if [[ "$(uname -s)" == Darwin && "$PROFILE_TYPE" == memory ]]; then
    maybe_switch_macos_memory_target
  fi
  ensure_target_running
  log "Target PID=$PID; type=$PROFILE_TYPE; duration=${DURATION}s; OS=$(uname -s)"
  case "$(uname -s)" in
  Linux)
    ensure_linux_tools
    if [[ "$PROFILE_TYPE" == cpu ]]; then
      ensure_flamegraph
      data="$OUTPUT_DIR/cpu-$PID-$timestamp.perf.data"
      folded="$OUTPUT_DIR/cpu-$PID-$timestamp.folded"
      svg="$OUTPUT_DIR/cpu-$PID-$timestamp.svg"
      log "Reproduce the workload now."
      perf record -F 199 -g --call-graph dwarf -p "$PID" -o "$data" -- sleep "$DURATION"
      perf script -i "$data" | "$FLAMEGRAPH_DIR/stackcollapse-perf.pl" > "$folded"
      "$FLAMEGRAPH_DIR/flamegraph.pl" --title "Cockpit CPU PID $PID" "$folded" > "$svg"
      log "CPU flame graph: $svg"
    else
      command -v heaptrack >/dev/null || die "heaptrack is unavailable after installation."
      log "Reproduce the workload now; heaptrack will detach after ${DURATION}s."
      heaptrack --pid "$PID" & collector=$!
      sleep "$DURATION"
      kill -INT "$collector" 2>/dev/null || true
      wait "$collector" || true
      latest="$(find "$(pwd)" "$OUTPUT_DIR" -maxdepth 1 -type f -name 'heaptrack.*.gz' -print0 2>/dev/null | xargs -0 ls -1t 2>/dev/null | head -1 || true)"
      [[ -n "$latest" ]] || die "heaptrack did not produce an output file. Check ptrace permissions."
      target="$OUTPUT_DIR/memory-$PID-$timestamp.heaptrack.gz"
      [[ "$latest" == "$target" ]] || mv -- "$latest" "$target"
      log "Allocation profile: $target (open with: heaptrack_gui '$target'; select Flame Graph)"
    fi
    ;;
  Darwin)
    ensure_macos_tools
    if [[ "$PROFILE_TYPE" == cpu ]]; then
      ensure_flamegraph
      raw="$OUTPUT_DIR/cpu-$PID-$timestamp.sample.txt"
      folded="$OUTPUT_DIR/cpu-$PID-$timestamp.folded"
      svg="$OUTPUT_DIR/cpu-$PID-$timestamp.svg"
      log "Reproduce the workload now."
      sample "$PID" "$DURATION" -file "$raw"
      collapse="$FLAMEGRAPH_DIR/stackcollapse-sample.awk"
      [[ -x "$collapse" || -f "$collapse" ]] || die "Installed FlameGraph lacks stackcollapse-sample.awk."
      awk -f "$collapse" "$raw" > "$folded"
      "$FLAMEGRAPH_DIR/flamegraph.pl" --title "Cockpit CPU PID $PID" "$folded" > "$svg"
      log "CPU flame graph: $svg"
    else
      ensure_macos_instruments_access
      trace="$OUTPUT_DIR/memory-$PID-$timestamp.trace"
      log "Reproduce the workload now. Keep Desktop PID $PID running until Instruments finishes."
      if ! run_privileged xcrun xctrace record --no-prompt --template Allocations --attach "$PID" --time-limit "${DURATION}s" --output "$trace"; then
        log "Instruments may have saved an incomplete trace at: $trace"
        die "Instruments could not attach to PID $PID. Confirm it is still running and grant Developer Tools access with --enable-developer-tools if macOS requests it."
      fi
      fix_output_ownership "$trace"
      log "Allocation trace: $trace (open in Instruments and choose the Call Tree/Flame Graph view)"
    fi
    ;;
    *) die "This shell entry point supports Linux/macOS. On Windows run tools/profile-desktop.ps1." ;;
  esac
done
