//! `SimulatorHandler` core: state, constructors, IPC request dispatch/routing,
//! and helpers shared by the sibling method groups in `lifecycle.rs`
//! (run lifecycle), `open_world.rs` (dynamic entity/agent-goal control),
//! and `control.rs` (action approval, plugins, replay/diff, recording
//! persistence, event cursor bookkeeping).
//!
//! Rust allows a struct's `impl` to be split across multiple files as long
//! as each is a sibling module of the struct's defining module; this file
//! keeps the type definition, construction, and dispatch routing, while the
//! sibling files above hold the actual command handlers. This split is a
//! pure reorganization of a single, previously very large `impl
//! SimulatorHandler` block; no method changed behavior as part of the split.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use cockpit_agent::{HumanAgentDriver, LocalMcpServer, RuleAgent};
use cockpit_plugin::{PluginExecutor, PluginFailure, PluginHost, PluginPolicy};
use cockpit_recording::{Recording, RecordingQueue, RecordingQueuePolicy, RecordingStore};
use cockpit_world::{
    PluginFailureRecord, Simulation, SimulationError, WorldSnapshot, clock::RunStatus,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::live_run::backend_impl::BackendSession;

use super::proto::{
    IPC_VERSION, IpcError, SimulatorCommand, SimulatorEvent, SimulatorRequest, SimulatorResponse,
};

pub(super) type HandlerResult = Result<Value, Box<IpcError>>;
pub const MAX_EVENT_HISTORY: usize = 2_048;

/// Shared cancellation handle kept outside the mutable simulator state so a
/// stop request can interrupt a live ACP turn while that state is locked.
#[derive(Clone, Default)]
pub struct LiveTurnControl {
    active: Arc<Mutex<Option<CancellationToken>>>,
}

impl LiveTurnControl {
    pub fn begin(&self) -> CancellationToken {
        let token = CancellationToken::new();
        if let Ok(mut active) = self.active.lock()
            && let Some(previous) = active.replace(token.clone())
        {
            previous.cancel();
        }
        token
    }

    pub fn cancel(&self) -> bool {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.as_ref().cloned())
            .is_some_and(|token| {
                token.cancel();
                true
            })
    }

    pub fn finish(&self) {
        if let Ok(mut active) = self.active.lock() {
            *active = None;
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.as_ref().cloned())
            .is_some_and(|token| token.is_cancelled())
    }
}

pub(super) fn read_recording(path: &str) -> Result<Recording, Box<IpcError>> {
    let bytes = fs::read(Path::new(path)).map_err(|error| {
        Box::new(IpcError {
            code: "RECORDING_READ_FAILED".to_string(),
            message: error.to_string(),
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "recording-diff".to_string(),
        })
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| Box::new(SimulatorHandler::serialization_error(error.to_string())))
}

pub struct SimulatorHandler {
    pub(super) session_token: String,
    pub(super) simulation: Option<Simulation>,
    pub(super) recording: Option<Recording>,
    pub(super) server: LocalMcpServer,
    pub(super) agent: RuleAgent,
    pub(super) live_driver: HumanAgentDriver,
    pub(super) live_backend: Option<BackendSession>,
    pub(super) events: Vec<SimulatorEvent>,
    pub(super) next_cursor: u64,
    pub(super) recording_store: Option<RecordingStore>,
    pub(super) plugin_host: PluginHost,
    pub(super) plugin_policy: PluginPolicy,
    pub(super) plugin_executors: BTreeMap<String, Box<dyn PluginExecutor>>,
    pub(super) recording_queue: RecordingQueue,
    pub(super) live_turn_control: LiveTurnControl,
}

impl SimulatorHandler {
    pub fn new(session_token: impl Into<String>) -> Self {
        Self::with_live_turn_control(session_token, LiveTurnControl::default())
    }

    pub fn with_live_turn_control(
        session_token: impl Into<String>,
        live_turn_control: LiveTurnControl,
    ) -> Self {
        Self {
            session_token: session_token.into(),
            simulation: None,
            recording: None,
            server: LocalMcpServer::default(),
            agent: RuleAgent::default(),
            live_driver: HumanAgentDriver::new(),
            live_backend: None,
            events: Vec::new(),
            next_cursor: 0,
            recording_store: None,
            plugin_host: PluginHost::default(),
            plugin_policy: PluginPolicy::default(),
            plugin_executors: BTreeMap::new(),
            recording_queue: RecordingQueue::new(256, RecordingQueuePolicy::FailRun),
            live_turn_control,
        }
    }

    pub fn live_turn_control(&self) -> LiveTurnControl {
        self.live_turn_control.clone()
    }

    pub fn session_token(&self) -> &str {
        &self.session_token
    }

    /// Return an immutable snapshot of the active recording for an external
    /// evaluator. The Simulator does not load rubrics or perform scoring.
    pub fn recording_snapshot(&self, run_id: &str) -> Option<Recording> {
        self.recording
            .as_ref()
            .filter(|recording| recording.run_id == run_id)
            .cloned()
    }

    pub fn new_persistent(
        session_token: impl Into<String>,
        database_path: &str,
    ) -> Result<Self, String> {
        let mut handler = Self::new(session_token);
        handler.recording_store =
            Some(RecordingStore::open(database_path).map_err(|error| error.to_string())?);
        Ok(handler)
    }

    pub async fn dispatch_async(&mut self, request: SimulatorRequest) -> SimulatorResponse {
        if !matches!(
            request.command,
            SimulatorCommand::CreateLiveSimulationRun { .. }
                | SimulatorCommand::ResumeLiveSimulation { .. }
                | SimulatorCommand::StepLiveSimulation
        ) {
            return self.dispatch(request);
        }

        let correlation_id = request.correlation_id.clone();
        if request.version != IPC_VERSION {
            return self.error_response(
                correlation_id,
                "IPC_VERSION_UNSUPPORTED",
                format!("supported IPC version is {IPC_VERSION}"),
            );
        }
        if request.session_token != self.session_token {
            return self.error_response(
                correlation_id,
                "SESSION_UNAUTHORIZED",
                "session token is invalid".to_string(),
            );
        }

        let result = match request.command {
            SimulatorCommand::CreateLiveSimulationRun { path, timeout_ms } => {
                self.create_live_run(&path, timeout_ms).await
            }
            SimulatorCommand::ResumeLiveSimulation {
                scenario_path,
                run_id,
                timeout_ms,
            } => {
                self.resume_live_run(&scenario_path, &run_id, timeout_ms)
                    .await
            }
            SimulatorCommand::StepLiveSimulation => self.step_live().await,
            _ => unreachable!("non-live commands return through dispatch"),
        };
        Self::response_from_result(correlation_id, result)
    }

    pub fn dispatch(&mut self, request: SimulatorRequest) -> SimulatorResponse {
        let correlation_id = request.correlation_id.clone();
        if request.version != IPC_VERSION {
            return self.error_response(
                correlation_id,
                "IPC_VERSION_UNSUPPORTED",
                format!("supported IPC version is {IPC_VERSION}"),
            );
        }
        if request.session_token != self.session_token {
            return self.error_response(
                correlation_id,
                "SESSION_UNAUTHORIZED",
                "session token is invalid".to_string(),
            );
        }

        let result = match request.command {
            SimulatorCommand::ValidateScenario { path } => self.validate(&path),
            SimulatorCommand::CreateSimulationRun { path } => self.create_run(&path),
            SimulatorCommand::CreateLiveSimulationRun { .. }
            | SimulatorCommand::ResumeLiveSimulation { .. }
            | SimulatorCommand::StepLiveSimulation
            | SimulatorCommand::CancelLiveTurn => Err(Box::new(IpcError {
                code: "ASYNC_COMMAND_REQUIRED".to_string(),
                message: "live backend commands require async dispatch".to_string(),
                details: None,
                run_id: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.run_id().to_string()),
                tick: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.snapshot.tick),
                correlation_id: correlation_id.clone(),
            })),
            SimulatorCommand::ResumeSimulation {
                scenario_path,
                run_id,
            } => self.resume_run(&scenario_path, &run_id),
            SimulatorCommand::SpawnEntity { entity } => self.spawn_entity(entity),
            SimulatorCommand::RemoveEntity { entity_id } => self.remove_entity(&entity_id),
            SimulatorCommand::AddAgentGoal {
                agent_id,
                description,
                priority,
            } => self.add_agent_goal(&agent_id, description, priority),
            SimulatorCommand::SetAgentGoalStatus {
                agent_id,
                goal_id,
                status,
            } => self.set_agent_goal_status(&agent_id, &goal_id, status),
            SimulatorCommand::WaitAgentUntil {
                agent_id,
                wake_tick,
            } => self.wait_agent_until(&agent_id, wake_tick),
            SimulatorCommand::GetOpenWorldRuntime => {
                serde_json::to_value(self.live_driver.open_world())
                    .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
            }
            SimulatorCommand::CheckpointOpenWorld => self.checkpoint_open_world(),
            SimulatorCommand::StartSimulation => self.start(),
            SimulatorCommand::PauseSimulation => self.pause(),
            SimulatorCommand::StepSimulation => self.step(),
            SimulatorCommand::StopSimulation => self.stop(),
            SimulatorCommand::ApproveAction { request_id } => self.approve_action(&request_id),
            SimulatorCommand::RejectAction { request_id, reason } => {
                self.reject_action(&request_id, reason.as_deref())
            }
            SimulatorCommand::CancelAgentTurn => self.cancel_agent_turn(),
            SimulatorCommand::SetApprovalRequired { required } => {
                self.set_approval_required(required)
            }
            SimulatorCommand::GetSimulationSnapshot => self.snapshot(),
            SimulatorCommand::GetSimulationEvents { cursor } => Ok(json!({
                "events": self.events_after(cursor),
                "nextCursor": self.next_cursor,
                "firstAvailableCursor": self.events.first().map(SimulatorEvent::cursor).unwrap_or(self.next_cursor),
                "resetRequired": self.cursor_reset_required(cursor)
            })),
            SimulatorCommand::GetAgentTrace => Ok(json!({
                "events": self
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        SimulatorEvent::SimulationToolCall { .. }
                            | SimulatorEvent::SimulationHumanTurn { .. }
                    ))
                    .collect::<Vec<_>>()
            })),
            SimulatorCommand::StartReplay {
                scenario_path,
                recording_path,
            } => self.start_replay(&scenario_path, &recording_path),
            SimulatorCommand::DiffRecordings {
                source_recording_path,
                candidate_recording_path,
            } => self.diff_recordings(&source_recording_path, &candidate_recording_path),
            SimulatorCommand::Ping { seq } => Ok(json!({ "pong": true, "seq": seq })),
        };

        Self::response_from_result(correlation_id, result)
    }

    fn response_from_result(correlation_id: String, result: HandlerResult) -> SimulatorResponse {
        match result {
            Ok(result) => SimulatorResponse {
                version: IPC_VERSION,
                correlation_id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => SimulatorResponse {
                version: IPC_VERSION,
                correlation_id: error.correlation_id.clone(),
                ok: false,
                result: None,
                error: Some(*error),
            },
        }
    }

    pub(super) fn emit(&mut self, event: SimulatorEvent) {
        self.next_cursor += 1;
        let cursor = self.next_cursor;
        let event = match event {
            SimulatorEvent::SimulationStateChanged { state, run_id, .. } => {
                SimulatorEvent::SimulationStateChanged {
                    cursor,
                    state,
                    run_id,
                }
            }
            SimulatorEvent::SimulationTickCommitted {
                run_id,
                tick,
                sim_time_ms,
                version,
                ..
            } => SimulatorEvent::SimulationTickCommitted {
                cursor,
                run_id,
                tick,
                sim_time_ms,
                version,
            },
            SimulatorEvent::SimulationEvent { event, .. } => {
                SimulatorEvent::SimulationEvent { cursor, event }
            }
            SimulatorEvent::SimulationToolCall { trace, .. } => {
                SimulatorEvent::SimulationToolCall { cursor, trace }
            }
            SimulatorEvent::SimulationHumanTurn {
                tick,
                backend,
                evidence,
                ..
            } => SimulatorEvent::SimulationHumanTurn {
                cursor,
                tick,
                backend,
                evidence,
            },
            SimulatorEvent::SimulationActionResult { result, .. } => {
                SimulatorEvent::SimulationActionResult { cursor, result }
            }
            SimulatorEvent::SimulationPluginFailure { failure, .. } => {
                SimulatorEvent::SimulationPluginFailure { cursor, failure }
            }
            SimulatorEvent::SimulationEvaluationUpdated { evaluation, .. } => {
                SimulatorEvent::SimulationEvaluationUpdated { cursor, evaluation }
            }
            SimulatorEvent::SimulationError { error, .. } => {
                SimulatorEvent::SimulationError { cursor, error }
            }
        };
        self.events.push(event);
        if self.events.len() > MAX_EVENT_HISTORY {
            let excess = self.events.len() - MAX_EVENT_HISTORY;
            self.events.drain(..excess);
        }
    }

    fn error_response(
        &self,
        correlation_id: String,
        code: &str,
        message: String,
    ) -> SimulatorResponse {
        SimulatorResponse {
            version: IPC_VERSION,
            correlation_id: correlation_id.clone(),
            ok: false,
            result: None,
            error: Some(IpcError {
                code: code.to_string(),
                message,
                details: None,
                run_id: None,
                tick: None,
                correlation_id,
            }),
        }
    }

    pub(super) fn tick_committed_event(snapshot: &WorldSnapshot) -> SimulatorEvent {
        SimulatorEvent::SimulationTickCommitted {
            cursor: 0,
            run_id: snapshot.run_id.clone(),
            tick: snapshot.tick,
            sim_time_ms: snapshot.sim_time_ms,
            version: snapshot.version,
        }
    }

    pub(super) fn no_run_error() -> IpcError {
        IpcError {
            code: "RUN_NOT_CREATED".to_string(),
            message: "create a simulation run first".to_string(),
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "simulator".to_string(),
        }
    }

    pub(super) fn simulation_error(
        error: SimulationError,
        simulation: Option<&Simulation>,
    ) -> IpcError {
        IpcError {
            code: "SIMULATION_ERROR".to_string(),
            message: error.to_string(),
            details: None,
            run_id: simulation.map(|value| value.run_id().to_string()),
            tick: simulation.map(|value| value.snapshot.tick),
            correlation_id: "simulator".to_string(),
        }
    }

    pub(super) fn serialization_error(message: String) -> IpcError {
        IpcError {
            code: "SERIALIZATION_ERROR".to_string(),
            message,
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "simulator".to_string(),
        }
    }

    pub(super) fn tool_error(error: cockpit_agent::ToolError) -> IpcError {
        IpcError {
            code: error.code,
            message: error.message,
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "simulator".to_string(),
        }
    }
}

pub(super) fn deadline_reached(status: RunStatus, tick: u64, deadline_tick: u64) -> bool {
    matches!(status, RunStatus::Running) && tick >= deadline_tick
}

pub(super) fn plugin_failure_record(failure: &PluginFailure) -> PluginFailureRecord {
    PluginFailureRecord {
        plugin_id: failure.plugin_id.clone(),
        version: failure.version.clone(),
        reason: failure.reason.clone(),
        decision: serde_json::to_string(&failure.decision)
            .unwrap_or_else(|_| "disablePlugin".to_string())
            .trim_matches('"')
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveTurnControl, SimulatorEvent, SimulatorHandler, deadline_reached};
    use cockpit_recording::run_rule_agent_recording;
    use cockpit_scenario::load_scenario;
    use cockpit_world::clock::RunStatus;

    #[test]
    fn deadline_completion_only_applies_to_running_simulations() {
        assert!(deadline_reached(RunStatus::Running, 16, 16));
        assert!(deadline_reached(RunStatus::Running, 17, 16));
        assert!(!deadline_reached(RunStatus::Running, 15, 16));
        assert!(!deadline_reached(RunStatus::Paused, 16, 16));
        assert!(!deadline_reached(RunStatus::Failed, 16, 16));
    }

    #[test]
    fn live_turn_control_cancels_without_simulator_state_access() {
        let control = LiveTurnControl::default();
        let token = control.begin();

        assert!(control.cancel());
        assert!(token.is_cancelled());

        control.finish();
        assert!(!control.cancel());
    }

    #[test]
    fn live_failure_emits_an_execution_failed_evaluation() {
        let scenario = load_scenario("../../scenarios/smoke-in-cockpit.yaml").expect("scenario");
        let recording = run_rule_agent_recording(
            "failed-live-evaluation",
            scenario.clone(),
            scenario.max_ticks + 1,
        )
        .expect("recording");
        let mut handler = SimulatorHandler::new("session");
        handler.recording = Some(recording);
        handler.emit_execution_failure_evaluation(&scenario, "backend timeout".to_string());

        let SimulatorEvent::SimulationEvaluationUpdated { evaluation, .. } =
            handler.events.last().expect("evaluation event")
        else {
            panic!("last event must be an evaluation update");
        };
        assert_eq!(evaluation["passed"], false);
        assert_eq!(evaluation["executionPassed"], false);
        assert_eq!(evaluation["executionError"], "backend timeout");
    }
}
