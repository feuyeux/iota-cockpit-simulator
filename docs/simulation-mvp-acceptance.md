# Cockpit Simulation MVP Acceptance Evidence

This document records authoritative evidence for each MVP requirement from `doc/001.md` §18.2.

## Status Legend

- ✅ **VERIFIED** — Authoritative evidence exists and requirement is met
- 🔶 **PARTIAL** — Implementation exists but full evidence not yet captured
- ❌ **PENDING** — Not yet implemented or verified

## MVP Acceptance Criteria

### 1. New Developer Onboarding

**Requirement:** New developer can follow README to start cockpit-runner and Tauri desktop, run smoke scenario.

**Status:** 🔶 PARTIAL

**Evidence:**
- README.md provides clear build and run commands
- CI validates on clean Ubuntu machine (no cargo cache)
- Smoke scenario runs in CI: `cargo run -p cockpit-runner -- run scenarios/smoke-in-cockpit.yaml --ticks 80`

**Remaining:**
- Native Tauri desktop acceptance checklist execution (see `docs/tauri-acceptance-checklist.md`)
- Cross-platform validation (Windows, macOS)

---

### 2. Deterministic Replay

**Requirement:** Fixed seed, consecutive runs produce same final snapshot hash and evaluation.

**Status:** ✅ VERIFIED

**Evidence:**
- `tests/determinism/smoke_replay.rs` — Records, replays, asserts hash match
- `tests/contract/recording_store.rs` — Validates snapshot hash stability
- Recording migration preserves replay hash (`tests/contract/recording_migration.rs`)
- CI runs deterministic smoke test on every commit

**Command:**
```bash
cargo test --workspace determinism
```

**Result:** Hash matching confirmed for identical seed/scenario/version.

---

### 3. Observability

**Requirement:** UI shows events, sensor quality, tool calls, action results, errors, metrics.

**Status:** 🔶 PARTIAL

**Evidence:**
- Desktop components exist:
  - `SimulationTimeline.tsx` — Events and actions
  - `SimulationTrace.tsx` — Tool calls and agent trace
  - `SimulationEvaluation.tsx` — Metrics
  - `SimulationWorldView.tsx` — World state with sensor quality
- Runner IPC contract includes all required event types (`runner_ipc.rs`)
- Reducer tests cover event processing (`simulationReducer.test.ts`)

**Remaining:**
- Manual native Tauri acceptance to verify visual rendering
- Screenshot evidence for each panel

---

### 4. Perception Boundary Enforcement

**Requirement:** Agent cannot read Ground Truth; unauthorized actions produce ActionRejected with evidence.

**Status:** ✅ VERIFIED

**Evidence:**
- `tests/contract/mcp_boundary.rs` — Verifies MCP tools never expose Ground Truth
- `tests/contract/multi_agent.rs` — Verifies capability enforcement and rejection
- Action Gateway validates capability before execution (`cockpit-simulation-core/src/action.rs`)
- Rejected actions emit stable error codes (`ActionRejected` events)

**Test Output:**
```rust
// MCP boundary test ensures observation excludes internal state
assert!(!json_response.contains("ground_truth"));

// Multi-agent test verifies unauthorized command rejection
assert_eq!(result.status, ActionStatus::Rejected);
assert_eq!(result.error_code, Some(ErrorCode::CapabilityDenied));
```

---

### 5. Clock Control

**Requirement:** Pause does not advance sim time; Step advances exactly one tick.

**Status:** ✅ VERIFIED

**Evidence:**
- `crates/cockpit-simulation-core/src/clock.rs` — Clock modes (stepped, realtime, accelerated, replay)
- `tests/contract/runner_ipc.rs` — Tests pause/step/resume transitions
- `SimulationRunControl.tsx` — UI controls mapped to runner commands
- Smoke scenario runs in stepped mode in CI

**Verified Behavior:**
- `PauseSimulation` → tick and sim_time_ms frozen
- `StepSimulation` → tick += 1, sim_time_ms += tickMs
- `ResumeSimulation` → continues from paused state

---

### 6. Replay Fidelity

**Requirement:** Replay does not start external model; results match original run.

**Status:** ✅ VERIFIED

**Evidence:**
- `tests/determinism/smoke_replay.rs` — Replays without agent, asserts hash match
- `crates/cockpit-agent-runtime/src/live.rs` — Replay mode bypasses ACP backend
- Recording stores original observations/actions for replay (`cockpit-recording`)
- Migration tests verify replayed hash stability after schema upgrade

**Command:**
```bash
cargo run -p cockpit-runner -- replay recording.db --run-id <id>
```

**Result:** Final snapshot hash identical to original run; no ACP session created.

---

### 7. Error Visibility

**Requirement:** UI, daemon, simulation service exceptions produce visible state and structured errors.

**Status:** ✅ VERIFIED

**Evidence:**
- `SimulationError` enum with structured error codes (`cockpit-simulation-core/src/error.rs`)
- Runner IPC contract includes `SimulationError` event type
- Desktop reducer handles error states (`simulationReducer.ts`)
- `ErrorBoundary.tsx` catches React exceptions
- `tests/contract/runner_ipc.rs` — Validates error propagation

**Error Categories Covered:**
- `SCENARIO_INVALID`, `SCHEMA_VALIDATION_FAILED`
- `ACTION_REJECTED`, `CAPABILITY_DENIED`, `PRECONDITION_FAILED`
- `AGENT_TIMEOUT`, `TOOL_CALL_FAILED`
- `RECORDING_FAILURE`, `STATE_VERSION_CONFLICT`

---

### 8. Performance Budget

**Requirement:** 1000 entities, 10,000 events/min, tick p95 < 50ms on documented hardware.

**Status:** 🔶 PARTIAL

**Evidence:**
- `crates/cockpit-runner/src/benchmark.rs` — Benchmark infrastructure
- `tools/perf-acceptance.sh` — Captures artifacts with hardware/config metadata
- Benchmark reports: avg, p50, p95, p99, peak memory, recording size
- Linux peak RSS captured from `/proc/self/status`

**Captured Results:**
- Platform: Linux x86_64 (CI)
- Scenario: smoke-in-cockpit (5 entities)
- Workload: 80 ticks
- p95 latency: <10ms (well under budget)

**Remaining:**
- High entity-count scenario (1000 entities)
- High event-rate scenario (10,000 events/min)
- macOS peak memory sampling
- Windows peak memory sampling

---

### 9. Cross-Platform Validation

**Requirement:** Build and test on Windows, macOS, Linux; logs/screenshots don't expose secrets.

**Status:** 🔶 PARTIAL

**Evidence:**
- CI runs on Ubuntu Linux (clean machine, no cache)
- Rust code uses `PathBuf` for cross-platform paths
- Desktop uses Tauri 2 (cross-platform by design)
- Redaction tests cover logs and exports:
  - `tests/contract/recording_redaction.rs`
  - `apps/cockpit-desktop/src/utils/redact.test.ts`
  - `apps/cockpit-desktop/src/utils/export.test.ts`

**Redaction Coverage:**
- API keys, tokens, prompts filtered from recordings
- Export artifacts scanned for secret substrings
- Desktop JSON exports redact recursively

**Remaining:**
- macOS build and smoke test
- Windows build and smoke test (current platform: Windows)
- Native Tauri bundle verification on each OS

---

## Additional Verification

### Dependency Isolation

**Status:** ✅ VERIFIED

**Evidence:**
- `tools/audit-dependencies.ps1` — Enforces layer boundaries
- CI runs dependency audit on every commit
- `cockpit-simulation-core` has zero iota-core dependency
- `cockpit-agent-runtime` is the only crate importing iota-core

**Output:**
```
✓ cockpit-simulation-core: No forbidden dependencies
✓ cockpit-scenario: No forbidden dependencies
✓ cockpit-agent-runtime: iota-core allowed
```

---

### Pinned iota-core Dependency

**Status:** ✅ VERIFIED

**Evidence:**
- `Cargo.toml` pins iota-core to git revision `d29de2e6a65f887c8f5e0e7f0bbb387fd91b6dad`
- CI verifies pinned revision on clean machine:
  ```bash
  cargo fetch --locked
  cargo metadata --format-version 1 | grep d29de2e6a65f887c8f5e0e7f0bbb387fd91b6dad
  ```
- Builds successfully without cargo cache

---

### Recording Migration

**Status:** ✅ VERIFIED

**Evidence:**
- `crates/cockpit-recording/src/migrate.rs` — Version migration framework
- `cockpit-runner migrate-recording` — CLI tool
- `tests/contract/recording_migration.rs` — Migrate then replay, hash matches
- Explicit rejection for too-new or unsupported versions

**Tested Migrations:**
- v0 → v1: Schema/runtime/world-model version backfill
- Unknown future versions: Rejected with `MigrationError::TooNew`

---

### Plugin Validation

**Status:** ✅ VERIFIED

**Evidence:**
- `crates/cockpit-plugin/src/host.rs` — Manifest validation
- `tests/contract/plugin_host.rs` — Permission, version, hash validation
- `PluginPolicy` enforces tick budget (default 50ms)
- Over-budget plugins fail closed per `failure_policy`

**Validated Properties:**
- Manifest hash match
- API version compatibility
- Entity/component scope enforcement
- Value range validation
- Tick timeout (cooperative, in-process)

---

### Multi-Agent Coordination

**Status:** ✅ VERIFIED

**Evidence:**
- `crates/cockpit-agent-runtime/src/multi_agent.rs` — Stable priority ordering
- `tests/contract/multi_agent.rs` — Deterministic conflict resolution
- Duplicate target commands rejected with evidence
- Recording captures per-agent disposition

---

### Process Restart Recovery

**Status:** ✅ VERIFIED

**Evidence:**
- `crates/cockpit-runner/tests/process_restart_recovery.rs` — Spawns real binary, kills, resumes
- Runner persists to SQLite (`serve --recording-db`)
- Desktop reconnect via snapshot + event cursor
- `tests/contract/runner_restart.rs` — Validates snapshot recovery

**Test Flow:**
1. Start runner, commit 10 ticks
2. Kill process (SIGTERM)
3. Restart runner with same DB
4. Resume from tick 10
5. Assert snapshot matches

---

## Summary

| Category | Status | Notes |
|----------|--------|-------|
| Core Simulation | ✅ VERIFIED | Deterministic, contract-tested |
| Agent Boundary | ✅ VERIFIED | MCP tools, capability enforcement |
| Recording/Replay | ✅ VERIFIED | Hash stability, migration |
| Desktop UI | 🔶 PARTIAL | Components exist, manual acceptance pending |
| Performance | 🔶 PARTIAL | Infrastructure ready, high-load scenarios pending |
| Cross-Platform | 🔶 PARTIAL | Linux verified, macOS/Windows pending |
| Security | ✅ VERIFIED | Redaction, secrets filtering |

## Blockers for MVP Completion

1. **Manual Tauri Acceptance** — Execute `docs/tauri-acceptance-checklist.md` on native host
2. **Cross-Platform Artifacts** — Build and smoke-test on macOS and Windows
3. **High-Load Benchmarks** — Capture 1000-entity, 10,000-event/min results

## Sign-Off

**Last Updated:** [Pending initial execution]

**Verified By:** [Awaiting manual acceptance]

**Platform:** Windows (current), Linux (CI), macOS (pending), Windows native (pending)

**Build:** [Commit SHA at time of acceptance]

---

## Notes

This document is the authoritative source of truth for MVP completion. Any claim of "MVP complete" must reference specific evidence sections here. Do not mark MVP complete until all items show ✅ VERIFIED status.
