//! `SimulatorHandler` methods for action approval/rejection, agent-turn
//! cancellation, plugin discovery/execution, snapshot/replay/diff
//! inspection, event cursor bookkeeping, and recording persistence.
//!
//! Split out of `handler.rs` to separate this control/inspection surface
//! from run lifecycle (`lifecycle.rs`) and open-world entity/agent control
//! (`open_world.rs`); this is a pure reorganization with no behavior
//! changes.

use std::{collections::BTreeMap, fs, path::Path};

use cockpit_plugin::{PluginExecutor, PluginFailure, PluginPolicy, PluginTickOutcome};
use cockpit_recording::{Recording, diff_recordings, replay_recording};
use cockpit_scenario::load_scenario;
use cockpit_world::{Simulation, StateDiff, clock::RunStatus};
use serde_json::{Value, json};

use super::handler::{HandlerResult, SimulatorHandler, plugin_failure_record, read_recording};
use super::proto::{IpcError, SimulatorEvent};

impl SimulatorHandler {
    pub fn configure_plugins(
        &mut self,
        directory: impl AsRef<Path>,
        policy: PluginPolicy,
        executors: BTreeMap<String, Box<dyn PluginExecutor>>,
    ) -> Vec<PluginFailure> {
        self.plugin_policy = policy;
        self.plugin_executors = executors;
        let failures = self.plugin_host.discover(directory, &self.plugin_policy);
        self.update_recording_plugin_hashes();
        for failure in &failures {
            self.emit_plugin_failure(failure);
        }
        failures
    }

    pub(super) fn update_recording_plugin_hashes(&mut self) {
        let hashes = self
            .plugin_host
            .manifests()
            .map(|manifest| format!("{}@{}:{}", manifest.id, manifest.version, manifest.hash))
            .collect();
        if let Some(recording) = self.recording.as_mut() {
            recording.plugin_hashes = hashes;
        }
    }

    pub(super) fn approve_action(&mut self, request_id: &str) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let result = self.server.approve_action(&mut simulation, request_id);
        self.simulation = Some(simulation);
        let result = result.map_err(|error| Box::new(Self::tool_error(error)))?;
        self.emit(SimulatorEvent::SimulationActionResult {
            cursor: 0,
            result: result.clone(),
        });
        serde_json::to_value(result)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
    }

    pub(super) fn reject_action(
        &mut self,
        request_id: &str,
        reason: Option<&str>,
    ) -> HandlerResult {
        let simulation = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let result = self
            .server
            .reject_action(simulation, request_id, false)
            .map_err(|error| Box::new(Self::tool_error(error)))?;
        self.emit(SimulatorEvent::SimulationActionResult {
            cursor: 0,
            result: result.clone(),
        });
        Ok(json!({
            "result": result,
            "reason": reason
        }))
    }

    pub(super) fn cancel_agent_turn(&mut self) -> HandlerResult {
        let simulation = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let results = self.server.cancel_pending_actions(simulation);
        for result in &results {
            self.emit(SimulatorEvent::SimulationActionResult {
                cursor: 0,
                result: result.clone(),
            });
        }
        Ok(json!({ "cancelled": true, "count": results.len() }))
    }

    pub(super) fn set_approval_required(&mut self, required: bool) -> HandlerResult {
        self.server.set_approval_required(required);
        Ok(json!({ "approvalRequired": required }))
    }

    pub(super) fn persist_recording(&mut self) -> HandlerResult {
        let Some(recording) = self.recording.as_ref() else {
            return Ok(Value::Null);
        };
        let Some(store) = self.recording_store.as_mut() else {
            return Ok(Value::Null);
        };
        store
            .save(recording)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))?;
        Ok(Value::Null)
    }

    /// Persist the recording without letting a transient storage failure
    /// (disk full, SQLite lock contention) discard the in-memory
    /// `Simulation`. Earlier callers used `self.persist_recording()?` at
    /// this point in the tick-commit path, before `self.simulation` had
    /// been written back from the local `simulation` variable taken at the
    /// top of the function. A storage error would propagate through `?`
    /// immediately, returning from the handler with `self.simulation` still
    /// `None` - permanently stranding the run: every subsequent command
    /// would see "no run in progress" even though the tick had already been
    /// committed in memory. This surfaces the failure as a
    /// `RECORDING_PERSIST_FAILED` event instead, so the frontend can inform
    /// the operator while the run remains controllable and the next
    /// successful persist can catch up.
    pub(super) fn emit_persist_recording_failure(&mut self, simulation: &Simulation, tick: u64) {
        if let Err(error) = self.persist_recording() {
            self.emit(SimulatorEvent::SimulationError {
                cursor: 0,
                error: IpcError {
                    code: "RECORDING_PERSIST_FAILED".to_string(),
                    message: error.message,
                    details: error.details,
                    run_id: Some(simulation.run_id().to_string()),
                    tick: Some(tick),
                    correlation_id: "recording-persist".to_string(),
                },
            });
        }
    }

    pub(super) fn snapshot(&self) -> HandlerResult {
        let simulation = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        serde_json::to_value(&simulation.snapshot)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
    }

    pub(super) fn start_replay(
        &mut self,
        scenario_path: &str,
        recording_path: &str,
    ) -> HandlerResult {
        let scenario = load_scenario(scenario_path)
            .map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        let bytes = fs::read(Path::new(recording_path)).map_err(|error| {
            Box::new(IpcError {
                code: "RECORDING_READ_FAILED".to_string(),
                message: error.to_string(),
                details: None,
                run_id: None,
                tick: None,
                correlation_id: "replay".to_string(),
            })
        })?;
        let recording: Recording = serde_json::from_slice(&bytes)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))?;
        let replay = replay_recording("replay-run", scenario.clone(), &recording)
            .map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        let mut simulation = Simulation::new(replay.run_id.clone(), scenario);
        simulation
            .start()
            .map_err(|error| Box::new(Self::simulation_error(error, Some(&simulation))))?;
        let actions_by_tick = recording.recorded_actions_by_tick();
        let state_diffs_by_tick = recording.recorded_state_diffs_by_tick();
        self.events.clear();
        self.next_cursor = 0;
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Replaying,
            run_id: Some(replay.run_id.clone()),
        });
        for source_tick in &recording.ticks {
            let actions = actions_by_tick
                .get(&source_tick.tick)
                .cloned()
                .unwrap_or_default();
            let state_diffs = state_diffs_by_tick
                .get(&source_tick.tick)
                .cloned()
                .unwrap_or_default();
            let step = simulation
                .step_with_recorded_inputs(actions, state_diffs)
                .map_err(|error| Box::new(Self::simulation_error(error, Some(&simulation))))?;
            let snapshot = simulation.snapshot.clone();
            self.emit(Self::tick_committed_event(&snapshot));
            for event in step.events {
                self.emit(SimulatorEvent::SimulationEvent { cursor: 0, event });
            }
            for trace in step.tool_calls {
                self.emit(SimulatorEvent::SimulationToolCall { cursor: 0, trace });
            }
            for result in step.action_results {
                self.emit(SimulatorEvent::SimulationActionResult { cursor: 0, result });
            }
        }
        simulation.status = RunStatus::Completed;
        self.simulation = Some(simulation);
        self.recording = Some(replay.clone());
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Completed,
            run_id: Some(replay.run_id.clone()),
        });
        Ok(json!({
            "runId": replay.run_id,
            "ticks": replay.ticks.len(),
            "finalSnapshotHash": replay.final_snapshot_hash()
        }))
    }

    pub(super) fn diff_recordings(
        &self,
        source_recording_path: &str,
        candidate_recording_path: &str,
    ) -> HandlerResult {
        let source = read_recording(source_recording_path)?;
        let candidate = read_recording(candidate_recording_path)?;
        serde_json::to_value(diff_recordings(&source, &candidate))
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
    }

    pub(super) fn events_after(&self, cursor: Option<u64>) -> Vec<SimulatorEvent> {
        let cursor = cursor.unwrap_or(0);
        self.events
            .iter()
            .filter(|event| event.cursor() > cursor)
            .cloned()
            .collect()
    }

    pub(super) fn cursor_reset_required(&self, cursor: Option<u64>) -> bool {
        let Some(cursor) = cursor else {
            return false;
        };
        let Some(first) = self.events.first().map(SimulatorEvent::cursor) else {
            return false;
        };
        cursor.saturating_add(1) < first
    }

    pub(super) fn run_plugins(
        &mut self,
        simulation: &Simulation,
    ) -> (Vec<StateDiff>, Vec<PluginFailure>) {
        let plugin_ids = self
            .plugin_host
            .plugin_ids()
            .filter(|id| self.plugin_executors.contains_key(*id))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut diffs = Vec::new();
        let mut failures = Vec::new();
        for plugin_id in plugin_ids {
            let Some(executor) = self.plugin_executors.get_mut(&plugin_id) else {
                continue;
            };
            match self.plugin_host.run_tick(
                &plugin_id,
                &simulation.snapshot,
                executor.as_mut(),
                &self.plugin_policy,
            ) {
                PluginTickOutcome::Accepted(plugin_diffs) => {
                    diffs.extend(plugin_diffs.into_iter().map(|diff| StateDiff {
                        source_id: diff.plugin_id,
                        entity_id: diff.entity_id,
                        component_path: diff.component_path,
                        value: diff.value,
                        expected_state_version: diff.expected_state_version,
                    }))
                }
                PluginTickOutcome::Failed(failure) => failures.push(failure),
            }
        }
        (diffs, failures)
    }

    pub(super) fn emit_plugin_failure(&mut self, failure: &PluginFailure) {
        self.emit(SimulatorEvent::SimulationPluginFailure {
            cursor: 0,
            failure: plugin_failure_record(failure),
        });
    }
}
