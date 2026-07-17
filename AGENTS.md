# Repository Guidelines

## Project Structure

This repository is an independent Rust workspace for the cockpit simulation runtime. Core domain logic is in `crates/cockpit-simulation-core`; scenario parsing, agent integration, recording, evaluation, plugins, and runner IPC live in their respective `crates/cockpit-*` packages. The Tauri 2 + React + TypeScript desktop client is under `apps/cockpit-desktop` (frontend in `src`, native host in `src-tauri`). Example scenarios are in `scenarios/`. Contract, determinism, and integration tests are in `tests/`.

## Build, Test, and Development Commands

Run commands from the repository root unless noted:

- `cargo test --workspace` runs all Rust unit and contract/integration tests.
- `cargo clippy --workspace --all-targets -- -D warnings` enforces warning-free Rust code.
- `cargo fmt --all --check` verifies formatting; use `cargo fmt --all` to apply it.
- `cargo run -p cockpit-runner -- --help` checks the runner CLI and available options.
- `npm test` in `apps/cockpit-desktop` runs the Vitest suite once.
- `npm run test:tsc` in `apps/cockpit-desktop` performs the strict TypeScript check without emitting files.
- `npm run build` in `apps/cockpit-desktop` creates the Vite production build.

Use `rtk` before shell commands when available, for example `rtk cargo test --workspace`.

## Coding Style and Naming

Use Rust 2024 edition conventions and let `rustfmt` determine layout. Prefer small, typed domain APIs over arbitrary JSON patches; preserve deterministic ordering and explicit versioning at boundaries. Use `snake_case` for Rust functions/modules, `PascalCase` for types and React components, and `camelCase` for serialized IPC/JSON fields. Keep the simulation core free of UI, network, ACP, and model dependencies. Add concise comments only for non-obvious invariants.

## Testing Guidelines

Add focused tests beside the affected contract: determinism tests belong in `tests/determinism`, boundary/failure tests in `tests/contract`, and end-to-end agent tests in `tests/integration`. Cover rejection paths, redaction, replay hashes, cursor reconnect behavior, and deterministic ordering when changing shared contracts. Run the full Rust and desktop gates before submitting changes.

## Commits and Pull Requests

Use short imperative commit subjects, such as `Integrate plugin execution into runner ticks` or `Recover desktop state after stale event cursors`. Keep commits focused and avoid unrelated formatting churn. Pull requests should explain behavior and affected boundaries, list verification commands, link the relevant issue or requirement, and include screenshots for desktop UI changes. Do not claim MVP completion unless every bundled benchmark scenario in [`docs/user-guide-zh.md`](docs/user-guide-zh.md) has been run end to end with a passing evaluation.

## Security and Architecture

Ground truth is owned by the simulation service; agents access only authorized sensor observations and typed Action Gateway commands. Never commit secrets, API keys, hidden reasoning, or unredacted traces. `iota-core` is the external integration boundary and must remain isolated to `cockpit-agent-runtime`; do not add dependencies on iota CLI, desktop, kanban, or daemon protocols. Plugin output is untrusted and must pass manifest, permission, scope, version, and StateDiff validation.
