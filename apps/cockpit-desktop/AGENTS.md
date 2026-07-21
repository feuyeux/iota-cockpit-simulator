# Repository Guidelines

## Project Structure & Module Organization

This directory contains the Tauri 2 desktop client for the cockpit simulation. React and TypeScript UI code lives in `src/`: components are under `src/components`, state transitions under `src/state`, IPC/client hooks under `src/hooks` and `src/simulatorClient.ts`, and shared domain types and utilities in `src/types`, `src/utils`, and `src/config`. Tests are colocated with the modules they cover and use the `.test.ts` or `.test.tsx` suffix. The Rust/Tauri host is in `src-tauri/src`; its sidecar preparation script is `src-tauri/prepare-sidecar.sh`, and packaging settings are in `src-tauri/tauri.conf.json`. `dist/` is generated output and should not be edited directly.

## Build, Test, and Development Commands

Run these commands from this directory (prefix with `rtk` when available):

- `npm run dev` starts the Vite frontend at `127.0.0.1:15342`.
- `npm test` runs the Vitest suite once; use `npm run test:watch` for interactive development.
- `npm run test:tsc` performs the strict TypeScript check without emitting files.
- `npm run build` type-checks and creates the Vite production bundle.
- `npm run tauri:dev` prepares the simulator sidecar and launches the desktop app. This is the canonical development launcher; it triggers Tauri's `beforeDevCommand`, which starts the Vite dev server on `127.0.0.1:15342`. From the repository root you can run `./run.sh` to do the same thing with extra diagnostics.
- `npm run tauri:build` prepares the sidecar and creates installable Tauri artifacts. The produced bundle embeds the frontend via `frontendDist`, so it does not need Vite at runtime.

For Rust workspace checks, run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` from the repository root.

## Coding Style & Naming Conventions

Use strict TypeScript with two-space indentation and the existing React functional-component style. Name components and types in `PascalCase`, functions and variables in `camelCase`, and files consistently with their primary export (for example, `SimulationTimeline.tsx`). Keep serialized IPC fields in `camelCase`. Let Rust `rustfmt` determine formatting and use `snake_case` for Rust modules and functions. Prefer typed state/actions and shared constants over ad hoc JSON or magic numbers.

## Testing Guidelines

Use Vitest with the `jsdom` environment. Add focused tests beside changed utilities, reducers, or components, including rejection and redaction paths where applicable. Run `npm test`, `npm run test:tsc`, and `npm run build` before submitting UI or IPC changes.

## Commit & Pull Request Guidelines

Use short imperative commit subjects, such as `Add replay export controls`. Keep commits focused. Pull requests should describe behavior and affected frontend/native boundaries, link the relevant issue or requirement, list verification commands, and include screenshots for desktop UI changes.

## Security & Configuration

Treat simulator/plugin output as untrusted. Preserve client-side redaction before exporting traces or recordings, never commit tokens or secrets, and keep simulator access behind the typed IPC boundary. Changes involving the sidecar should be tested from the repository root because `prepare-sidecar.sh` builds and stages `cockpit-simulator`.
