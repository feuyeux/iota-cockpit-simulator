//! `SimulatorHandler` methods for simulation run lifecycle: scenario
//! validation, run creation (offline and live-backed), start/pause/step,
//! resume from a persisted recording, and stop.
//!
//! Split out of `handler.rs` to separate run lifecycle from open-world
//! entity/agent control (`open_world.rs`) and action/plugin/recording
//! control (`control.rs`); this is a pure reorganization with no behavior
//! changes.

use cockpit_agent::{HumanAgentDriver, LocalMcpServer, RuleAgent};
use cockpit_plugin::{PluginFailurePolicy, PluginHost};
use cockpit_recording::{Recording, RecordingQueue, RecordingQueueOutcome, RecordingQueuePolicy};
use cockpit_scenario::load_scenario;
use cockpit_world::{Simulation, SimulationScenario, clock::RunStatus};
use serde_json::json;

use crate::live_run::backend_impl::backend_session;

use super::handler::{HandlerResult, SimulatorHandler, deadline_reached, plugin_failure_record};
use super::proto::{IpcError, SimulatorEvent};

impl SimulatorHandler {
    pub(super) fn validate(&self, path: &str) -> HandlerResult {
        let scenario =
            load_scenario(path).map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        Ok(json!({
            "id": scenario.id,
            "path": path,
            "schemaVersion": scenario.schema_version,
            "scenarioHash": scenario.scenario_hash,
            "seed": scenario.seed,
            "agentId": scenario.agent.agent_id
        }))
    }

    pub(super) fn create_run(&mut self, path: &str) -> HandlerResult {
        let scenario =
            load_scenario(path).map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        let run_id = format!("run-{}", scenario.id);
        self.simulation = Some(Simulation::new(run_id.clone(), scenario.clone()));
        self.recording = Some(Recording::new(run_id.clone(), &scenario));
        self.server = LocalMcpServer::default();
        self.agent = RuleAgent::default();
        self.live_driver = HumanAgentDriver::new();
        self.live_backend = None;
        self.plugin_host = PluginHost::default();
        self.plugin_executors.clear();
        self.recording_queue = RecordingQueue::new(256, RecordingQueuePolicy::FailRun);
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Ready,
            run_id: Some(run_id.clone()),
        });
        self.persist_recording()?;
        Ok(json!({
            "runId": run_id,
            "status": RunStatus::Ready,
            "scenarioHash": scenario.scenario_hash
        }))
    }

    pub(super) async fn create_live_run(&mut self, path: &str, timeout_ms: u64) -> HandlerResult {
        let scenario =
            load_scenario(path).map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        let run_id = format!("live-run-{}", scenario.id);
        let reuse_ready_run = self.simulation.as_ref().is_some_and(|simulation| {
            simulation.run_id() == run_id.as_str()
                && simulation.status == RunStatus::Ready
                && simulation.snapshot.tick == 0
                && simulation.scenario.scenario_hash.as_str() == scenario.scenario_hash.as_str()
        }) && self
            .live_backend
            .as_ref()
            .is_some_and(|backend| backend.timeout_ms() == timeout_ms);
        if reuse_ready_run {
            let backend_label = self
                .live_backend
                .as_ref()
                .expect("reuse requires an existing live backend")
                .label();
            eprintln!("live run create reused ready backend: run_id={run_id}");
            return Ok(json!({
                "runId": run_id,
                "status": RunStatus::Ready,
                "scenarioHash": scenario.scenario_hash,
                "backend": backend_label
            }));
        }
        if let Some(mut previous_backend) = self.live_backend.take() {
            eprintln!("live backend shutdown before replacing active run");
            previous_backend.shutdown().await;
        }
        let mut backend = backend_session(&scenario, timeout_ms).map_err(|error| {
            Box::new(IpcError {
                code: "LIVE_BACKEND_INIT_FAILED".to_string(),
                message: error.to_string(),
                details: None,
                run_id: None,
                tick: None,
                correlation_id: "live-backend".to_string(),
            })
        })?;
        backend.warm().await.map_err(|error| {
            Box::new(IpcError {
                code: "LIVE_BACKEND_INIT_FAILED".to_string(),
                message: format!("Hermes ACP warm-up failed: {error}"),
                details: None,
                run_id: None,
                tick: None,
                correlation_id: "live-backend".to_string(),
            })
        })?;
        let backend_label = backend.label();
        self.simulation = Some(Simulation::new(run_id.clone(), scenario.clone()));
        self.recording = Some(Recording::new(run_id.clone(), &scenario));
        self.server = LocalMcpServer::default();
        self.agent = RuleAgent::default();
        self.live_driver = HumanAgentDriver::new();
        self.live_backend = Some(backend);
        self.plugin_host = PluginHost::default();
        self.plugin_executors.clear();
        self.recording_queue = RecordingQueue::new(256, RecordingQueuePolicy::FailRun);
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Ready,
            run_id: Some(run_id.clone()),
        });
        self.persist_recording()?;
        Ok(json!({
            "runId": run_id,
            "status": RunStatus::Ready,
            "scenarioHash": scenario.scenario_hash,
            "backend": backend_label
        }))
    }

    pub(super) async fn resume_live_run(
        &mut self,
        scenario_path: &str,
        run_id: &str,
        timeout_ms: u64,
    ) -> HandlerResult {
        let store = self.recording_store.as_ref().ok_or_else(|| {
            Box::new(IpcError {
                code: "RECORDING_STORE_UNAVAILABLE".to_string(),
                message: "persistent recording store is not configured".to_string(),
                details: None,
                run_id: Some(run_id.to_string()),
                tick: None,
                correlation_id: "live-resume".to_string(),
            })
        })?;
        let recording = store
            .load(run_id)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))?;
        let checkpoint = recording.open_world_checkpoint.as_ref().ok_or_else(|| {
            Box::new(IpcError {
                code: "OPEN_WORLD_CHECKPOINT_MISSING".to_string(),
                message: "recording does not contain a live open-world checkpoint".to_string(),
                details: None,
                run_id: Some(run_id.to_string()),
                tick: recording.ticks.last().map(|tick| tick.tick),
                correlation_id: "live-resume".to_string(),
            })
        })?;
        let scenario = load_scenario(scenario_path)
            .map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        if scenario.scenario_hash != recording.scenario_hash {
            return Err(Box::new(IpcError {
                code: "SCENARIO_HASH_MISMATCH".to_string(),
                message: "recording scenario hash does not match the requested scenario"
                    .to_string(),
                details: None,
                run_id: Some(run_id.to_string()),
                tick: Some(checkpoint.world.tick),
                correlation_id: "live-resume".to_string(),
            }));
        }

        let mut simulation = Simulation::new(run_id.to_string(), scenario.clone());
        let mut driver = HumanAgentDriver::new();
        let checkpoint_bytes = checkpoint
            .encode()
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))?;
        driver
            .restore_checkpoint(&mut simulation, &checkpoint_bytes)
            .map_err(|error| Box::new(Self::serialization_error(error)))?;
        simulation.status = RunStatus::Paused;

        if let Some(mut previous_backend) = self.live_backend.take() {
            eprintln!("live backend shutdown before resuming another run");
            previous_backend.shutdown().await;
        }
        let mut backend = backend_session(&scenario, timeout_ms).map_err(|error| {
            Box::new(IpcError {
                code: "LIVE_BACKEND_INIT_FAILED".to_string(),
                message: error.to_string(),
                details: None,
                run_id: Some(run_id.to_string()),
                tick: Some(simulation.snapshot.tick),
                correlation_id: "live-resume".to_string(),
            })
        })?;
        backend
            .restore_backend_sessions(driver.open_world())
            .await
            .map_err(|error| {
                Box::new(IpcError {
                    code: "ACP_SESSION_RESTORE_FAILED".to_string(),
                    message: error,
                    details: None,
                    run_id: Some(run_id.to_string()),
                    tick: Some(simulation.snapshot.tick),
                    correlation_id: "live-resume-acp".to_string(),
                })
            })?;
        backend.warm().await.map_err(|error| {
            Box::new(IpcError {
                code: "LIVE_BACKEND_INIT_FAILED".to_string(),
                message: format!("Hermes ACP warm-up failed: {error}"),
                details: None,
                run_id: Some(run_id.to_string()),
                tick: Some(simulation.snapshot.tick),
                correlation_id: "live-resume".to_string(),
            })
        })?;
        let backend_label = backend.label();

        self.events.clear();
        self.next_cursor = 0;
        self.simulation = Some(simulation);
        self.recording = Some(recording);
        self.server = LocalMcpServer::default();
        self.agent = RuleAgent::default();
        self.live_driver = driver;
        self.live_backend = Some(backend);
        self.plugin_host = PluginHost::default();
        self.plugin_executors.clear();
        self.recording_queue = RecordingQueue::new(256, RecordingQueuePolicy::FailRun);
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Paused,
            run_id: Some(run_id.to_string()),
        });
        Ok(json!({
            "runId": run_id,
            "tick": self.simulation.as_ref().map(|value| value.snapshot.tick).unwrap_or(0),
            "status": RunStatus::Paused,
            "backend": backend_label,
            "restoredAgents": self.live_driver.open_world().sessions.len()
        }))
    }

    pub(super) fn start(&mut self) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let result = simulation.start();
        if let Err(error) = result {
            let ipc_error = Self::simulation_error(error, Some(&simulation));
            self.simulation = Some(simulation);
            return Err(Box::new(ipc_error));
        }
        let run_id = simulation.run_id().to_string();
        self.simulation = Some(simulation);
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Running,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({ "runId": run_id, "status": RunStatus::Running }))
    }

    pub(super) fn resume_run(&mut self, scenario_path: &str, run_id: &str) -> HandlerResult {
        let store = self.recording_store.as_ref().ok_or_else(|| {
            Box::new(IpcError {
                code: "RECORDING_STORE_UNAVAILABLE".to_string(),
                message: "persistent recording store is not configured".to_string(),
                details: None,
                run_id: Some(run_id.to_string()),
                tick: None,
                correlation_id: "resume".to_string(),
            })
        })?;
        let recording = store
            .load(run_id)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))?;
        let scenario = load_scenario(scenario_path)
            .map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        let mut simulation = Simulation::new(run_id.to_string(), scenario.clone());
        simulation
            .start()
            .map_err(|error| Box::new(Self::simulation_error(error, Some(&simulation))))?;
        let actions_by_tick = recording.recorded_actions_by_tick();
        let state_diffs_by_tick = recording.recorded_state_diffs_by_tick();
        self.events.clear();
        self.next_cursor = 0;
        self.recording = Some(Recording::new(run_id.to_string(), &scenario));
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
            if let Some(target) = self.recording.as_mut() {
                target.push(step.clone());
            }
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
        self.simulation = Some(simulation);
        self.server = LocalMcpServer::default();
        self.agent = RuleAgent::default();
        Ok(json!({
            "runId": run_id,
            "tick": self.simulation.as_ref().map(|value| value.snapshot.tick).unwrap_or(0),
            "cursor": self.next_cursor,
            "status": RunStatus::Paused
        }))
    }

    pub(super) fn pause(&mut self) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let result = simulation.pause();
        if let Err(error) = result {
            let ipc_error = Self::simulation_error(error, Some(&simulation));
            self.simulation = Some(simulation);
            return Err(Box::new(ipc_error));
        }
        let run_id = simulation.run_id().to_string();
        self.simulation = Some(simulation);
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Paused,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({ "runId": run_id, "status": RunStatus::Paused }))
    }

    pub(super) fn step(&mut self) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        if simulation.status == RunStatus::Completed {
            let run_id = simulation.run_id().to_string();
            let tick = simulation.snapshot.tick;
            self.simulation = Some(simulation);
            return Ok(json!({
                "runId": run_id,
                "tick": tick,
                "status": RunStatus::Completed,
                "alreadyCompleted": true
            }));
        }
        let (plugin_diffs, plugin_failures) = self.run_plugins(&simulation);
        let result =
            self.agent
                .step_with_state_diffs(&mut simulation, &mut self.server, plugin_diffs);
        let step = match result {
            Ok(step) => step,
            Err(error) => {
                let ipc_error = Self::simulation_error(error, Some(&simulation));
                self.simulation = Some(simulation);
                return Err(Box::new(ipc_error));
            }
        };
        let mut step = step;
        step.plugin_failures = plugin_failures.iter().map(plugin_failure_record).collect();
        if plugin_failures
            .iter()
            .any(|failure| failure.decision == PluginFailurePolicy::PauseRun)
        {
            simulation.status = RunStatus::Paused;
        }
        if plugin_failures
            .iter()
            .any(|failure| failure.decision == PluginFailurePolicy::FailRun)
        {
            simulation.fail();
        }
        let plugin_status = simulation.status;
        let tick = step.tick;
        let snapshot = simulation.snapshot.clone();
        let snapshot_hash = step.snapshot_hash.clone();
        let queue_outcome = self.recording_queue.push(step.clone());
        match queue_outcome {
            RecordingQueueOutcome::Enqueued => {
                for queued_step in self.recording_queue.drain() {
                    if let Some(recording) = self.recording.as_mut() {
                        recording.push(queued_step);
                    }
                }
            }
            RecordingQueueOutcome::Dropped => {}
            RecordingQueueOutcome::Paused => simulation.status = RunStatus::Paused,
            RecordingQueueOutcome::Failed => simulation.fail(),
        }
        if matches!(
            queue_outcome,
            RecordingQueueOutcome::Paused | RecordingQueueOutcome::Failed
        ) {
            let health = self.recording_queue.health();
            self.emit(SimulatorEvent::SimulationError {
                cursor: 0,
                error: IpcError {
                    code: "RECORDING_QUEUE_OVERFLOW".to_string(),
                    message: "recording queue reached its bounded capacity".to_string(),
                    details: serde_json::to_value(health).ok(),
                    run_id: Some(simulation.run_id().to_string()),
                    tick: Some(tick),
                    correlation_id: "recording-queue".to_string(),
                },
            });
        }
        self.emit_persist_recording_failure(&simulation, tick);
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
        for failure in &step.plugin_failures {
            self.emit(SimulatorEvent::SimulationPluginFailure {
                cursor: 0,
                failure: failure.clone(),
            });
        }
        if !plugin_failures.is_empty()
            && matches!(plugin_status, RunStatus::Paused | RunStatus::Failed)
        {
            self.emit(SimulatorEvent::SimulationStateChanged {
                cursor: 0,
                state: plugin_status,
                run_id: Some(simulation.run_id().to_string()),
            });
        }
        if let Some(recording) = self.recording.as_ref() {
            let evaluation = json!({
                "status": "pending",
                "evaluator": "cockpit-evaluator",
                "recordingRunId": recording.run_id,
                "recordedTicks": recording.ticks.len()
            });
            self.emit(SimulatorEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation,
            });
        }
        if deadline_reached(simulation.status, tick, simulation.scenario.max_ticks) {
            simulation.status = RunStatus::Completed;
            self.emit(SimulatorEvent::SimulationStateChanged {
                cursor: 0,
                state: RunStatus::Completed,
                run_id: Some(simulation.run_id().to_string()),
            });
        }
        let run_id = simulation.run_id().to_string();
        let status = simulation.status;
        self.simulation = Some(simulation);
        Ok(json!({
            "runId": run_id,
            "tick": tick,
            "snapshotHash": snapshot_hash,
            "status": status,
            "recordingQueue": self.recording_queue.health()
        }))
    }

    pub(super) async fn step_live(&mut self) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        if simulation.status == RunStatus::Completed {
            let run_id = simulation.run_id().to_string();
            let tick = simulation.snapshot.tick;
            self.simulation = Some(simulation);
            return Ok(json!({
                "runId": run_id,
                "tick": tick,
                "status": RunStatus::Completed,
                "alreadyCompleted": true
            }));
        }
        let mut backend = self.live_backend.take().ok_or_else(|| {
            Box::new(IpcError {
                code: "LIVE_BACKEND_NOT_CREATED".to_string(),
                message: "create a live simulation run first".to_string(),
                details: None,
                run_id: Some(simulation.run_id().to_string()),
                tick: Some(simulation.snapshot.tick),
                correlation_id: "live-backend".to_string(),
            })
        })?;
        let backend_label = backend.label();
        let cancellation = self.live_turn_control.begin();
        backend.set_turn_cancellation(cancellation);
        let result = self
            .live_driver
            .step_with_tools(&mut simulation, &mut backend, &mut self.server)
            .await;
        let cancelled = self.live_turn_control.is_cancelled();
        self.live_turn_control.finish();
        self.live_backend = Some(backend);

        if cancelled {
            simulation.stop();
            let run_id = simulation.run_id().to_string();
            self.emit(SimulatorEvent::SimulationStateChanged {
                cursor: 0,
                state: RunStatus::Stopped,
                run_id: Some(run_id.clone()),
            });
            self.simulation = Some(simulation);
            return Ok(json!({
                "runId": run_id,
                "status": RunStatus::Stopped,
                "cancelled": true,
                "backend": backend_label,
            }));
        }

        let (step, human_turns) = match result {
            Ok(result) => result,
            Err(error) => {
                simulation.fail();
                let checkpoint = self.live_driver.checkpoint(&simulation);
                if let Some(recording) = self.recording.as_mut() {
                    recording.open_world_checkpoint = Some(checkpoint);
                }
                self.emit_persist_recording_failure(&simulation, simulation.snapshot.tick);
                self.emit_execution_failure_evaluation(&simulation.scenario, error.to_string());
                let ipc_error = IpcError {
                    code: "LIVE_BACKEND_TURN_FAILED".to_string(),
                    message: error.to_string(),
                    details: None,
                    run_id: Some(simulation.run_id().to_string()),
                    tick: Some(simulation.snapshot.tick),
                    correlation_id: "live-backend".to_string(),
                };
                self.emit(SimulatorEvent::SimulationError {
                    cursor: 0,
                    error: ipc_error.clone(),
                });
                self.emit(SimulatorEvent::SimulationStateChanged {
                    cursor: 0,
                    state: RunStatus::Failed,
                    run_id: Some(simulation.run_id().to_string()),
                });
                self.simulation = Some(simulation);
                return Err(Box::new(ipc_error));
            }
        };

        let tick = step.tick;
        let snapshot_hash = step.snapshot_hash.clone();
        let snapshot = simulation.snapshot.clone();
        let checkpoint = self.live_driver.checkpoint(&simulation);
        if let Some(recording) = self.recording.as_mut() {
            recording.push(step.clone());
            recording.push_human_turns(human_turns.clone());
            recording.open_world_checkpoint = Some(checkpoint);
        }
        self.emit_persist_recording_failure(&simulation, tick);
        self.emit(Self::tick_committed_event(&snapshot));
        for evidence in &human_turns {
            self.emit(SimulatorEvent::SimulationHumanTurn {
                cursor: 0,
                tick,
                backend: backend_label.to_string(),
                evidence: evidence.clone(),
            });
        }
        for event in step.events {
            self.emit(SimulatorEvent::SimulationEvent { cursor: 0, event });
        }
        for trace in step.tool_calls {
            self.emit(SimulatorEvent::SimulationToolCall { cursor: 0, trace });
        }
        for result in step.action_results {
            self.emit(SimulatorEvent::SimulationActionResult { cursor: 0, result });
        }
        if let Some(recording) = self.recording.as_ref() {
            let evaluation = json!({
                "status": "pending",
                "evaluator": "cockpit-evaluator",
                "recordingRunId": recording.run_id,
                "recordedTicks": recording.ticks.len()
            });
            self.emit(SimulatorEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation,
            });
        }
        if deadline_reached(simulation.status, tick, simulation.scenario.max_ticks) {
            simulation.status = RunStatus::Completed;
            self.emit(SimulatorEvent::SimulationStateChanged {
                cursor: 0,
                state: RunStatus::Completed,
                run_id: Some(simulation.run_id().to_string()),
            });
        }
        let run_id = simulation.run_id().to_string();
        let status = simulation.status;
        self.simulation = Some(simulation);
        Ok(json!({
            "runId": run_id,
            "tick": tick,
            "snapshotHash": snapshot_hash,
            "status": status,
            "backend": backend_label,
            "humanTurns": human_turns.len()
        }))
    }

    pub(super) fn stop(&mut self) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        simulation.stop();
        let run_id = simulation.run_id().to_string();
        self.simulation = Some(simulation);
        self.emit(SimulatorEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Stopped,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({ "runId": run_id, "status": RunStatus::Stopped }))
    }

    pub(super) fn emit_execution_failure_evaluation(
        &mut self,
        _scenario: &SimulationScenario,
        error: String,
    ) {
        if let Some(recording) = self.recording.as_ref() {
            let evaluation = json!({
                "status": "pending",
                "passed": false,
                "executionPassed": false,
                "evaluator": "cockpit-evaluator",
                "recordingRunId": recording.run_id,
                "recordedTicks": recording.ticks.len(),
                "executionError": error
            });
            self.emit(SimulatorEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation,
            });
        }
    }
}
