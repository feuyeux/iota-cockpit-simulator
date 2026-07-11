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
- `cockpit-runner` exposes a versioned tagged IPC contract with session authentication and event cursors for reconnect recovery.
- Recording headers carry runtime/world-model versions, application commit, plugin hashes, scenario hash, seed, and clock configuration; rejected actions publish stable `ActionRejected` events.
- Side-effecting MCP requests can be held as `pendingApproval` and controlled through `ApproveAction`, `RejectAction`, and `CancelAgentTurn` IPC commands.
- `apps/cockpit-desktop` is an independent React 19 + Vite 7 + TypeScript + Tailwind 4 + Lucide app with a Tauri 2 host, typed runner state, controls, world, timeline, trace, and evaluation panels.

The iota-core dependency is pinned to git revision `4d8a72a0af4a156437f7a23cfacbb059f0ee62e3`, with `default-features = false`; it is used only by `cockpit-agent-runtime`. The adapter is compile-tested and prompt-isolation tested; external backend startup remains opt-in and is not required for deterministic runs.

## Verify

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo metadata --format-version 1
cargo run -p cockpit-runner -- validate scenarios/smoke-in-cockpit.yaml
cargo run -p cockpit-runner -- run scenarios/smoke-in-cockpit.yaml --ticks 80
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
