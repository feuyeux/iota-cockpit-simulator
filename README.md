# iota-cockpit

An independent cockpit world simulation desktop application and agent runtime.

The project uses a new Tauri 2, React 19, TypeScript, Vite, Tailwind, and Lucide desktop application. It reuses only iota-core as an external Rust library dependency; it does not reuse iota-cli, iota-desktop, iota-kanban, or the iota daemon.

## Current implementation

This repository implements the complete local runtime, simulation, recording, independent evaluation, and Tauri host architecture:

- Rust workspace with pure simulation core, scenario loading, SQLite recording/replay, evaluation, `cockpit-agent`, and `cockpit-simulator`.
- `scenarios/` contains ten composite benchmark cockpit scenarios. Together they cover a 14-domain cockpit taxonomy spanning safety, comfort, sensing, driver and occupant monitoring, health, multimodal HMI, infotainment, personalization, navigation, energy, connectivity, ADAS, and cybersecurity. Every scenario closes through a typed Action Gateway command and traceable evaluation evidence; see `docs/user-guide-zh.md` for the per-scenario operating walkthrough with sequence and flow diagrams.
- `cockpit-agent` exposes eight typed simulation tools, including bounded human-owned Goal/wait controls, capability enforcement, a RuleAgent, a timeout-only execution policy, and an iota-core SkillRegistry adapter.
- `cockpit-agent` also has an `IotaCoreAcpAdapter` that builds a persona/goal prompt with human-scoped simulation tool schemas, omits the eager world observation, feeds completed tool exchanges into later rounds, and maps iota-core runtime events into a redacted trace.
- `cockpit-simulator run-live` drives one mandatory tool loop per human per tick through `HumanAgentDriver`. With the real `live-acp` backend, iota-core registers the hidden `cockpit-simulator mcp-bridge` stdio server in ACP `session/new`; native tool calls are captured in a private per-turn transaction view, replayed with the same call IDs through `LocalMcpServer`, and committed only when bridge and parent results agree. Synthetic and recording replay retain the equivalent JSON `toolCall` compatibility transport. Both end with a `final` disposition and enforce call-count, weighted tool-cost, wall-clock, response-size, pagination, capability, and action-result ownership boundaries. Actions are accepted only through `simulation.request_action`; a backend failure or divergence aborts without committing partial world/action state. There is no semantic fallback or circuit breaker.
- The desktop Live surface is model-only: it creates an `iota-core-acp` live run, keeps a distinct parked/restored ACP adapter, conversation, and private MCP state file for every human (including dynamically spawned humans), and requires one completed tool loop per scheduled human on every step. The model starts without an eager complete observation, chooses human-scoped read tools on demand, submits physical mutations through the typed Action Gateway tool, and receives tool results before finalizing the turn. All eleven domain commands resolve to typed `EffectPlan` operations and evidence in the componentized Effect Kernel; `Simulation` contains no legacy domain-action apply branch. The Rust `RuleAgent` path remains available for the offline CLI, Simulator protocol contracts, and deterministic tests, but it is not exposed as a desktop mode or fallback. Scenario YAML defines initial state plus deterministic `faults`/`influences`; those rules drive reproducible external risks, not successful domain interventions. The simulation core remains the sole authority for validation, physical evolution, `cockpitSystems` updates, and state commits. Live human-turn and tool-call evidence is emitted through IPC with free-form prose redacted. Manual approval and auto-step are not exposed in the desktop live UI.
- `cockpit-world::digital_twin` replaces the former scalar drift rules with a coupled two-zone vehicle model: calibrated RC thermodynamics, water-vapour balance, barometric pressure/leakage, smoke/CO₂/CO mass conservation with Beer-Lambert visibility, and two-node occupant thermoregulation/exposure. Aggregate thermal coefficients are reproducibly fitted from 1,302 closed-sedan observations in Mendeley Data DOI `10.17632/8mfgd8w9rg.1`; the 30% recursive holdout RMSE is `2.026942°C` versus a `2.916170°C` persistence baseline. An independent NIST Fire Calorimetry Database profile (DOI `10.18434/mds2-2314`) drives heat, soot and CO from a hash-gated 6,468-row full-scale ICE-minivan HRR trace and measured species yields; its 10-second lookup achieves `56.276435 kW` non-anchor holdout RMSE versus `109.328921 kW` persistence. The CFK-derived AL=2 COHb exposure/recovery model is independently field-validated in 100 armored-vehicle crew members (peak RMSE `1.94%`). Two-node thermoregulation now includes humidity-limited evaporative heat loss and is independently gated for one-hour rest stability, passive seated hot-vs-moderate direction, and the published 23–71% RH ordering; those sweat/heat-transfer parameters remain engineering values rather than a cohort fit. The exterior-fire-to-closed-cabin transfer, fire-soot applicability, pressure equalization, cohort generalization and remaining thermal-physiology parameter boundaries remain explicitly unfitted.
- `cockpit-simulator` exposes a versioned tagged IPC contract with session authentication and event cursors for reconnect recovery.
- Recording headers carry runtime/world-model versions, application commit, plugin hashes, scenario hash, seed, and clock configuration; world-model version 8 includes humidity-coupled multi-zone environment/physiology, measured-profile combustion age/HRR, authoritative climate, assistance, occupant, experience, mobility, connectivity, and cybersecurity state, and rejected actions publish stable `ActionRejected` events.
- `cockpit-evaluator` is the independent evaluation plane. It reads an immutable Recording JSON or opens a Simulator SQLite store with `RecordingStore::open_read_only`, loads a private rubric from `evaluations/private/`, and emits `pass`/`fail`/`inconclusive` with tick/entity/event evidence plus input, rubric, prompt, model, and schema hashes. It accepts either pre-recorded `--judge-a`/`--judge-b` decisions or launches two canonical-path-distinct `--judge-a-command`/`--judge-b-command` providers as isolated bounded subprocesses; per-provider `--judge-a-arg`/`--judge-b-arg` values pin identity, model, workspace, and transport configuration. The workspace includes concrete `cockpit-judge-hermes` and `cockpit-judge-opencode` executables: each invokes a real model through an ephemeral iota-core ACP session, accepts no credentials on argv/stdin, strictly parses a bare model JSON decision, and attaches trusted provenance outside the model. Judge identity, concrete model, prompt/rubric/schema provenance, timeout/output limits, and every recording citation are validated; duplicate identity, duplicate model, or any disagreement is inconclusive and fails the release gate by default. Simulator simulation paths publish only an evaluation `pending` marker and contain no scoring policy, rule registry, or private rubric reader.
- `evaluations/suite.yaml` is the default ten-scenario CI suite. `cockpit-evaluator --suite` executes scenario cases through a separate Simulator process (or reads immutable JSON/SQLite recordings), optionally applies the same two Judge providers, detects pass-to-non-pass baseline regressions, writes JSON and JUnit reports, and returns exit code 2 when the aggregate release gate fails. The Tauri host likewise launches `cockpit-evaluator` as a sidecar, persists evidence reports under the application data directory, and exposes one-click evaluation, Judge agreement, export, and history without moving private rubrics into Simulator or the React process.
- Public `scenarios/*.yaml` files are strict initialization resources: entities, agents, public non-scoring goals, runtime horizon, external faults, and influences only. `evaluation`, `deadlineTick`, rule IDs, thresholds, action mappings, and release gates are rejected by the parser and live exclusively under `evaluations/private/`, which is consumed only by `cockpit-evaluator`.
- `OpenWorldRuntime` gives every dynamic human an independent versioned session with Goal/Plan/Skill/Tool lifecycle state, bounded episodic recall, a bounded typed ACP conversation history and latest backend session identity, evolving relationship scores, weighted per-agent budgets, deterministic priority scheduling, wait/wake/recovery/replan transitions, and retired-entity tracking. Fresh iota-core logical sessions prevent cwd-based cross-human inheritance; after restart, each persisted backend session ID is reattached before any prompt using ACP `session/resume` or capability-gated `session/load` with the complete cwd/MCP set. Missing restore capability, shared cross-human session identity, or a backend mismatch fails the resume command with `ACP_SESSION_RESTORE_FAILED`; typed response summaries are used only when no native backend session has ever existed. `simulation.add_goal` and `simulation.wait_until` mutate only the authenticated human's runtime state. Session-authenticated IPC v4 additionally exposes dynamic entity creation/removal, Goal status, wait, runtime inspection, and checkpoint operations. `Simulation::spawn_entity`/`remove_entity` mutate the authoritative typed world with state-version advancement and emit `EntitySpawned`/`EntityRemoved` evidence. `OpenWorldCheckpoint` sleeps/restores the complete `WorldSnapshot` plus all agent sessions. Persistent IPC runs atomically store the latest checkpoint with each Recording; `ResumeLiveSimulation` restores world/control/conversation context and recreates isolated ACP transports. Native MCP state is mode `0600` and verified before reads on Unix. Windows creates each state generation with a protected `D:P(A;;FA;;;OW)` owner-rights DACL, atomically replaces it with `MoveFileExW`, and verifies the resulting SDDL before reads; platforms that are neither Unix nor Windows fail closed before writing Ground Truth.
- Side-effecting MCP requests can be held as `pendingApproval` and controlled through `ApproveAction`, `RejectAction`, and `CancelAgentTurn` IPC commands.
- `cockpit-plugin` validates manifest hash/API/permissions and gates candidate StateDiffs by entity, component path, state version, and value range.
- Multiple scenario agent grants are supported; `MultiAgentCoordinator` applies stable priority/agent ordering and rejects duplicate target commands deterministically.
- Recording payloads are content-addressed SHA-256 files with SQLite storing only payload hashes and sizes.
- Persistent simulator mode saves each committed tick and supports `ResumeSimulation` after process restart, restoring snapshot and event cursor state.
- `cockpit-simulator bench` reports average/p50/p95/p99/peak tick latency, recording size, fixed scenario hash/seed, and the requested entity/event workload.
- `apps/cockpit-desktop` is an independent React 19 + Vite 7 + TypeScript + Tailwind 4 + Lucide app with a Tauri 2 host, typed simulator state, and a 1600 × 900 focus workspace: scenario controls, the world view, and the realtime activity feed remain visible together, while evaluation and narrative open on demand in an insights drawer.

The workspace uses the local sibling `iota-sympantos-core` crate with `default-features = false`; it is consumed by `cockpit-agent` and the isolated Judge provider. The dependency exposes cancellable turns, non-persisting ephemeral evaluation sessions, and capability-gated ACP `session/resume`/`session/load`. External backend startup and credentials remain explicit deployment concerns and are not required for deterministic offline runs.

## Verify

```bash
python3 calibration/calibrate.py
python3 calibration/calibrate_vehicle_fire.py
python3 calibration/validate_human_heat_stress.py
python3 calibration/verify.py
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo metadata --format-version 1
cargo run -p cockpit-simulator -- validate scenarios/smoke-in-cockpit.yaml
for scenario in scenarios/*.yaml; do cargo run -q -p cockpit-simulator -- validate "$scenario" || exit 1; done
cargo run -p cockpit-simulator -- run scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-simulator -- run-live scenarios/smoke-in-cockpit.yaml --ticks 80
cargo run -p cockpit-simulator --features live-acp -- run-live scenarios/smoke-in-cockpit.yaml --ticks 80
pwsh ./tools/audit-dependencies.ps1
cd apps/cockpit-desktop
npm ci
npm test
npm run test:tsc
npm run build
```

## Run the complete benchmark evaluation suite

Build Simulator and evaluator, then execute all ten deterministic scenarios through the independent process boundary:

```bash
cargo build -p cockpit-simulator -p cockpit-evaluator
./target/debug/cockpit-evaluator \
  --suite evaluations/suite.yaml \
  --simulator-command ./target/debug/cockpit-simulator \
  --json-report target/evaluation-report.json \
  --junit-report target/evaluation-junit.xml
```

Use `--baseline <prior-report.json>` to reject any case that regresses from a passing release gate, and `--minimum-pass-rate <0..1>` to configure the aggregate gate. The default is `1.0`. Add the same paired `--judge-a-command` / `--judge-b-command` options shown below to run two independent model Judges for every suite case. Suite cases may alternatively reference an immutable `recording` JSON or `recordingDb` + `runId`; only scenario cases launch Simulator.

## Run two concrete model Judges

Build the two path-distinct provider binaries and create two dedicated, non-simulation workspaces. Credentials stay in each backend's normal protected configuration/environment; never pass them through evaluator/provider arguments.

```bash
cockpit-evaluator --recording run.json --rubric evaluations/private/smoke-in-cockpit.yaml \
  --judge-a-command target/release/cockpit-judge-hermes \
  --judge-a-arg=--judge-id --judge-a-arg=judge-hermes-a \
  --judge-a-arg=--model --judge-a-arg=claude-sonnet-4 \
  --judge-a-arg=--provider --judge-a-arg=anthropic \
  --judge-a-arg=--workspace --judge-a-arg=/path/to/private/judge-a \
  --judge-b-command target/release/cockpit-judge-opencode \
  --judge-b-arg=--judge-id --judge-b-arg=judge-opencode-b \
  --judge-b-arg=--model --judge-b-arg=gpt-5 \
  --judge-b-arg=--workspace --judge-b-arg=/path/to/private/judge-b \
  --judge-b-arg=--backend-command --judge-b-arg=/path/to/installed/opencode \
  --judge-b-arg=--backend-arg --judge-b-arg=acp
```

Both provider executable paths, Judge IDs, and model names must differ. The evaluator rejects unsupported evidence, provider timeout/output violations, duplicate identities/models, deterministic disagreement, and model disagreement.

For desktop one-click evaluation, the native host can be configured with paired `COCKPIT_JUDGE_A_BIN` / `COCKPIT_JUDGE_B_BIN` paths and JSON string arrays in `COCKPIT_JUDGE_A_ARGS_JSON` / `COCKPIT_JUDGE_B_ARGS_JSON`; `COCKPIT_JUDGE_TIMEOUT_MS` is optional. Do not place credentials in these values. When the pair is absent, the desktop runs the deterministic private-rubric plane and clearly reports that Judges were not configured.

## Run the desktop shell

```bash
cd apps/cockpit-desktop
npm run dev -- --host 127.0.0.1 --port 15342
```

Open <http://127.0.0.1:15342>.

For the native Tauri host, set `COCKPIT_SIMULATOR_BIN` to a `cockpit-simulator` executable to force the isolated loopback process during development, then use `npm run tauri:dev`. Without that variable, debug builds use the embedded handler. Packaged release builds automatically discover the bundled `cockpit-simulator` sidecar next to the desktop executable; `COCKPIT_SIMULATOR_BIN` still takes precedence.

Packaging ships Simulator and evaluator as separate Tauri sidecars: `npm run tauri:build` (and `tauri:dev`) first runs `src-tauri/prepare-sidecar.sh`, which builds both release binaries and stages `cockpit-simulator-<target-triple>` plus `cockpit-evaluator-<target-triple>` under `src-tauri/binaries/`. Private rubrics are bundled as native resources and are never exposed to Simulator or the webview. The launched Simulator process persists to a recording database (`serve --recording-db`), so it recovers its snapshot and event cursor across a real process restart; this is verified by `crates/cockpit-simulator/tests/process_restart_recovery.rs`.

Desktop bundles use the generated PNG/ICNS/ICO set derived from `src-tauri/icons/cockpit-icon.svg`. In a headless macOS session, Finder may time out while cosmetically arranging a DMG; rerunning the generated create-dmg command with `--skip-jenkins` produces the same application payload without custom icon positioning.
