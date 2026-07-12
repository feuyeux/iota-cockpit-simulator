# iota-cockpit-simulator

An independent cockpit world simulation desktop application and agent runtime.

The project uses a new Tauri 2, React 19, TypeScript, Vite, Tailwind, and Lucide desktop application. It reuses only iota-core as an external Rust library dependency; it does not reuse iota-cli, iota-desktop, iota-kanban, or the iota daemon.

The complete product, architecture, interaction model, implementation plan, and acceptance criteria are documented in [doc/001.md](doc/001.md).

## Current implementation

This repository currently implements the Phase 0 slice, the Phase 1 local agent/runtime boundary, and the Phase 2 Tauri host:

- Rust workspace with pure simulation core, scenario loading, SQLite recording/replay, evaluation, `cockpit-agent-runtime`, and `cockpit-runner`.
- `scenarios/smoke-in-cockpit.yaml` drives smoke detection, a scripted shutdown action, recording, replay, and evaluation.
- `cockpit-agent-runtime` exposes six typed simulation tools, capability enforcement, a RuleAgent, timeout/fallback policy, and an iota-core SkillRegistry adapter.
- `cockpit-agent-runtime` also has an `IotaCoreAcpAdapter` that builds an observation-only prompt and maps iota-core runtime events into a redacted trace.
- `cockpit-runner run-live` drives an advisory live agent turn per tick through the retry/circuit-breaker policy with a RuleAgent fallback, records completed/fallback disposition evidence per tick, and always commits the deterministic tick. The real iota-core backend is opt-in behind the `live-acp` cargo feature; the default build uses a synthetic backend so deterministic runs stay offline.
- `cockpit-runner` exposes a versioned tagged IPC contract with session authentication and event cursors for reconnect recovery.
- Recording headers carry runtime/world-model versions, application commit, plugin hashes, scenario hash, seed, and clock configuration; rejected actions publish stable `ActionRejected` events.
- Side-effecting MCP requests can be held as `pendingApproval` and controlled through `ApproveAction`, `RejectAction`, and `CancelAgentTurn` IPC commands.
- `cockpit-plugin` validates manifest hash/API/permissions and gates candidate StateDiffs by entity, component path, state version, and value range.
- Multiple scenario agent grants are supported; `MultiAgentCoordinator` applies stable priority/agent ordering and rejects duplicate target commands deterministically.
- Recording payloads are content-addressed SHA-256 files with SQLite storing only payload hashes and sizes.
- Persistent runner mode saves each committed tick and supports `ResumeSimulation` after process restart, restoring snapshot and event cursor state.
- `cockpit-runner bench` reports average/p50/p95/p99/peak tick latency, recording size, fixed scenario hash/seed, and the requested entity/event workload.
- `apps/cockpit-desktop` is an independent React 19 + Vite 7 + TypeScript + Tailwind 4 + Lucide app with a Tauri 2 host, typed runner state, controls, world, timeline, trace, and evaluation panels.

The iota-core dependency is pinned to git revision `d29de2e6a65f887c8f5e0e7f0bbb387fd91b6dad`, with `default-features = false`; it is used only by `cockpit-agent-runtime`. This revision exposes the cancellable turn API (`run_cancellable` + `TurnCancelled`), which the runtime policy, ACP adapter, and live driver use to cancel a live turn mid-flight and record a distinct `Cancelled` disposition. The adapter is compile-tested and prompt-isolation tested; external backend startup remains opt-in and is not required for deterministic runs.

## Verify

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo metadata --format-version 1
cargo run -p cockpit-runner -- validate scenarios/smoke-in-cockpit.yaml
cargo run -p cockpit-runner -- run scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-runner -- run-live scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-runner --features live-acp -- run-live scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-runner -- migrate-recording recording.json --dry-run
pwsh ./tools/audit-dependencies.ps1
cd apps/cockpit-desktop
npm install
npm test
npm run build
```

## Run the desktop shell

```bash
cd apps/cockpit-desktop
npm run dev -- --host 127.0.0.1 --port 15342
```

Open <http://127.0.0.1:15342>.

For the native Tauri host, set `COCKPIT_RUNNER_BIN` to the `cockpit-runner` executable to launch the isolated loopback runner process, then use `npm run tauri:dev`; package with `npm run tauri:build`. Without that variable, development uses the embedded handler explicitly.

Packaging ships the runner as a Tauri sidecar: `npm run tauri:build` (and `tauri:dev`) first runs `src-tauri/prepare-sidecar.sh`, which builds `cockpit-runner --release` and stages it as `src-tauri/binaries/cockpit-runner-<target-triple>` for the `externalBin` bundle. The launched runner process persists to a recording database (`serve --recording-db`), so it recovers its snapshot and event cursor across a real process restart; this is verified by `crates/cockpit-runner/tests/process_restart_recovery.rs`.
