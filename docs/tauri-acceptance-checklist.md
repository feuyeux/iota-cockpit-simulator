# Native Tauri Acceptance Checklist

Manual acceptance for the packaged desktop shell against a real `cockpit-runner`
process. Automated coverage exists for the reducer, guards, storage, reconnect,
export/redaction, and the `SimulationWorldView` component; the items below
require a human on the native host and are not yet automated.

## Setup

1. Build the runner: `cargo build --release -p cockpit-runner`.
2. Export `COCKPIT_RUNNER_BIN` to the built `cockpit-runner` executable.
3. From `apps/cockpit-desktop`: `npm run tauri:dev` (or run the packaged binary
   from `npm run tauri:build`).

## Checklist

- [ ] **Connect / idle**: launching the app reaches `connectedIdle`; the header
      shows the service as connected.
- [ ] **Scenario load**: browse to `scenarios/smoke-in-cockpit.yaml` via the
      native file dialog; the app reaches `ready` with the scenario hash shown.
- [ ] **Run / pause / step / stop**: controls are enabled per the state guards
      (`canStart`/`canPause`/`canStep`/`canStop`) and drive the runner.
- [ ] **Approval flow**: with approval mode on, a side-effecting action is held
      as `pendingApproval`; Approve applies it and Reject records a rejection.
- [ ] **Replay**: replay a saved recording; the timeline reproduces committed
      ticks and the final snapshot hash matches the source run.
- [ ] **Reconnect reset**: kill and restart the runner process; the desktop
      refreshes from the authoritative snapshot (`snapshotReset`) before
      applying retained events, with no stale ticks.
- [ ] **Error state**: force a `RECORDING_QUEUE_OVERFLOW` (small queue capacity)
      and confirm the UI shows the `failed` state with the structured error.
- [ ] **Loading state**: scenario validation in flight shows the loading state
      and disables run controls.
- [ ] **Empty state**: before any run, timeline/trace/evaluation panels show
      empty-state messaging rather than blank panels.
- [ ] **Export redaction**: export events/traces/actions; confirm no secret
      fields (apiKey/token/prompt/etc.) appear in the downloaded artifacts.

## Result

Record the OS, app version, runner commit, and pass/fail per item. File any
failure as an issue linked to the relevant requirement in `doc/001.md`.
