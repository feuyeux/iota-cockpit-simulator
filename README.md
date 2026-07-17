# iota-cockpit-simulator

An independent cockpit world simulation desktop application and agent runtime.

The project uses a new Tauri 2, React 19, TypeScript, Vite, Tailwind, and Lucide desktop application. It reuses only iota-core as an external Rust library dependency; it does not reuse iota-cli, iota-desktop, iota-kanban, or the iota daemon.

## Current implementation

This repository currently implements the Phase 0 slice, the Phase 1 local agent/runtime boundary, and the Phase 2 Tauri host:

- Rust workspace with pure simulation core, scenario loading, SQLite recording/replay, evaluation, `cockpit-agent-runtime`, and `cockpit-runner`.
- `scenarios/` contains ten composite benchmark cockpit scenarios. Together they cover a 14-domain cockpit taxonomy spanning safety, comfort, sensing, driver and occupant monitoring, health, multimodal HMI, infotainment, personalization, navigation, energy, connectivity, ADAS, and cybersecurity. Every scenario closes through a typed Action Gateway command and traceable evaluation evidence; see `docs/user-guide-zh.md` for the per-scenario operating walkthrough with sequence and flow diagrams.
- `cockpit-agent-runtime` exposes six typed simulation tools, capability enforcement, a RuleAgent, a minimal timeout-only execution policy, and an iota-core SkillRegistry adapter.
- `cockpit-agent-runtime` also has an `IotaCoreAcpAdapter` that builds an observation-only prompt and maps iota-core runtime events into a redacted trace.
- `cockpit-runner run-live` drives one mandatory backend turn per human per tick through `HumanAgentDriver`: every human's decision (utterance, actions, internal state delta, narrative) must come from a real backend call. There is no fallback, retry, or circuit breaker; a backend failure, timeout, or invalid output aborts the run immediately and is reported as an error rather than silently substituted. The real iota-core backend is opt-in behind the `live-acp` cargo feature; the default build uses an explicit synthetic backend (labeled `"synthetic"` in every report) so deterministic runs stay offline without pretending to exercise a real backend.
- The desktop Live surface is model-only: it creates an `iota-core-acp` live run, keeps one backend session, and requires one backend turn per human on every step. The Rust `RuleAgent` path remains available for the offline CLI, Runner protocol contracts, and deterministic tests, but it is not exposed as a desktop mode or fallback. Scenario YAML defines initial state plus deterministic `faults`/`influences`; those rules drive reproducible external risks, not successful domain interventions. The model must select a typed command, and the simulation core remains the sole authority for validation, physical evolution, `cockpitSystems` updates, and state commits. Live human-turn evidence is emitted through IPC with free-form prose redacted. Manual approval and auto-step are not exposed in the desktop live UI.
- `cockpit-runner` exposes a versioned tagged IPC contract with session authentication and event cursors for reconnect recovery.
- Recording headers carry runtime/world-model versions, application commit, plugin hashes, scenario hash, seed, and clock configuration; world-model version 4 includes authoritative climate, assistance, occupant, experience, mobility, connectivity, and cybersecurity state, and rejected actions publish stable `ActionRejected` events.
- Side-effecting MCP requests can be held as `pendingApproval` and controlled through `ApproveAction`, `RejectAction`, and `CancelAgentTurn` IPC commands.
- `cockpit-plugin` validates manifest hash/API/permissions and gates candidate StateDiffs by entity, component path, state version, and value range.
- Multiple scenario agent grants are supported; `MultiAgentCoordinator` applies stable priority/agent ordering and rejects duplicate target commands deterministically.
- Recording payloads are content-addressed SHA-256 files with SQLite storing only payload hashes and sizes.
- Persistent runner mode saves each committed tick and supports `ResumeSimulation` after process restart, restoring snapshot and event cursor state.
- `cockpit-runner bench` reports average/p50/p95/p99/peak tick latency, recording size, fixed scenario hash/seed, and the requested entity/event workload.
- `apps/cockpit-desktop` is an independent React 19 + Vite 7 + TypeScript + Tailwind 4 + Lucide app with a Tauri 2 host, typed runner state, and a 1600 × 900 focus workspace: scenario controls, the world view, and the realtime activity feed remain visible together, while evaluation and narrative open on demand in an insights drawer.

The iota-core dependency is pinned to git revision `d29de2e6a65f887c8f5e0e7f0bbb387fd91b6dad`, with `default-features = false`; it is used only by `cockpit-agent-runtime`. This revision exposes the cancellable turn API (`run_cancellable` + `TurnCancelled`), which the runtime policy, ACP adapter, and live driver use to cancel a live turn mid-flight and record a distinct `Cancelled` disposition. The adapter is compile-tested and prompt-isolation tested; external backend startup remains opt-in and is not required for deterministic runs.

## Verify

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo metadata --format-version 1
cargo run -p cockpit-runner -- validate scenarios/smoke-in-cockpit.yaml
for scenario in scenarios/*.yaml; do cargo run -q -p cockpit-runner -- validate "$scenario" || exit 1; done
cargo run -p cockpit-runner -- run scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-runner -- run-live scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-runner --features live-acp -- run-live scenarios/smoke-in-cockpit.yaml --ticks 80
pwsh ./tools/audit-dependencies.ps1
cd apps/cockpit-desktop
npm ci
npm test
npm run test:tsc
npm run build
```

## Run the desktop shell

```bash
cd apps/cockpit-desktop
npm run dev -- --host 127.0.0.1 --port 15342
```

Open <http://127.0.0.1:15342>.

For the native Tauri host, set `COCKPIT_RUNNER_BIN` to a `cockpit-runner` executable to force the isolated loopback process during development, then use `npm run tauri:dev`. Without that variable, debug builds use the embedded handler. Packaged release builds automatically discover the bundled `cockpit-runner` sidecar next to the desktop executable; `COCKPIT_RUNNER_BIN` still takes precedence.

Packaging ships the runner as a Tauri sidecar: `npm run tauri:build` (and `tauri:dev`) first runs `src-tauri/prepare-sidecar.sh`, which builds `cockpit-runner --release` and stages it as `src-tauri/binaries/cockpit-runner-<target-triple>` for the `externalBin` bundle. The launched runner process persists to a recording database (`serve --recording-db`), so it recovers its snapshot and event cursor across a real process restart; this is verified by `crates/cockpit-runner/tests/process_restart_recovery.rs`.

Desktop bundles use the generated PNG/ICNS/ICO set derived from `src-tauri/icons/cockpit-icon.svg`. In a headless macOS session, Finder may time out while cosmetically arranging a DMG; rerunning the generated create-dmg command with `--skip-jenkins` produces the same application payload without custom icon positioning.
