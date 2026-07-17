use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use cockpit_agent_runtime::{HumanAgentDriver, LocalMcpServer, RuleAgent};
use cockpit_plugin::{
    PluginExecutor, PluginFailure, PluginFailurePolicy, PluginHost, PluginPolicy, PluginTickOutcome,
};
use cockpit_recording::{
    Recording, RecordingQueue, RecordingQueueOutcome, RecordingQueuePolicy, RecordingStore,
    diff_recordings, replay_recording,
};
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::{
    PluginFailureRecord, Simulation, SimulationError, SimulationScenario, StateDiff, WorldSnapshot,
    clock::RunStatus,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::live_run::backend_impl::{BackendSession, backend_session};

use super::proto::{
    IPC_VERSION, IpcError, RunnerCommand, RunnerEvent, RunnerRequest, RunnerResponse,
};

type HandlerResult = Result<Value, Box<IpcError>>;
pub const MAX_EVENT_HISTORY: usize = 2_048;

/// Shared cancellation handle kept outside the mutable runner state so a
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

fn read_recording(path: &str) -> Result<Recording, Box<IpcError>> {
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
        .map_err(|error| Box::new(RunnerHandler::serialization_error(error.to_string())))
}

pub struct RunnerHandler {
    session_token: String,
    simulation: Option<Simulation>,
    recording: Option<Recording>,
    server: LocalMcpServer,
    agent: RuleAgent,
    live_driver: HumanAgentDriver,
    live_backend: Option<BackendSession>,
    events: Vec<RunnerEvent>,
    next_cursor: u64,
    recording_store: Option<RecordingStore>,
    plugin_host: PluginHost,
    plugin_policy: PluginPolicy,
    plugin_executors: BTreeMap<String, Box<dyn PluginExecutor>>,
    recording_queue: RecordingQueue,
    live_turn_control: LiveTurnControl,
}

impl RunnerHandler {
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

    fn update_recording_plugin_hashes(&mut self) {
        let hashes = self
            .plugin_host
            .manifests()
            .map(|manifest| format!("{}@{}:{}", manifest.id, manifest.version, manifest.hash))
            .collect();
        if let Some(recording) = self.recording.as_mut() {
            recording.plugin_hashes = hashes;
        }
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

    pub async fn dispatch_async(&mut self, request: RunnerRequest) -> RunnerResponse {
        if !matches!(
            request.command,
            RunnerCommand::CreateLiveSimulationRun { .. } | RunnerCommand::StepLiveSimulation
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
            RunnerCommand::CreateLiveSimulationRun { path, timeout_ms } => {
                self.create_live_run(&path, timeout_ms).await
            }
            RunnerCommand::StepLiveSimulation => self.step_live().await,
            _ => unreachable!("non-live commands return through dispatch"),
        };
        Self::response_from_result(correlation_id, result)
    }

    pub fn dispatch(&mut self, request: RunnerRequest) -> RunnerResponse {
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
            RunnerCommand::ValidateScenario { path } => self.validate(&path),
            RunnerCommand::CreateSimulationRun { path } => self.create_run(&path),
            RunnerCommand::CreateLiveSimulationRun { .. }
            | RunnerCommand::StepLiveSimulation
            | RunnerCommand::CancelLiveTurn => Err(Box::new(IpcError {
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
            RunnerCommand::ResumeSimulation {
                scenario_path,
                run_id,
            } => self.resume_run(&scenario_path, &run_id),
            RunnerCommand::StartSimulation => self.start(),
            RunnerCommand::PauseSimulation => self.pause(),
            RunnerCommand::StepSimulation => self.step(),
            RunnerCommand::StopSimulation => self.stop(),
            RunnerCommand::ApproveAction { request_id } => self.approve_action(&request_id),
            RunnerCommand::RejectAction { request_id, reason } => {
                self.reject_action(&request_id, reason.as_deref())
            }
            RunnerCommand::CancelAgentTurn => self.cancel_agent_turn(),
            RunnerCommand::SetApprovalRequired { required } => self.set_approval_required(required),
            RunnerCommand::GetSimulationSnapshot => self.snapshot(),
            RunnerCommand::GetSimulationEvents { cursor } => Ok(json!({
                "events": self.events_after(cursor),
                "nextCursor": self.next_cursor,
                "firstAvailableCursor": self.events.first().map(RunnerEvent::cursor).unwrap_or(self.next_cursor),
                "resetRequired": self.cursor_reset_required(cursor)
            })),
            RunnerCommand::GetAgentTrace => Ok(json!({
                "events": self
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        RunnerEvent::SimulationToolCall { .. }
                            | RunnerEvent::SimulationHumanTurn { .. }
                    ))
                    .collect::<Vec<_>>()
            })),
            RunnerCommand::StartReplay {
                scenario_path,
                recording_path,
            } => self.start_replay(&scenario_path, &recording_path),
            RunnerCommand::DiffRecordings {
                source_recording_path,
                candidate_recording_path,
            } => self.diff_recordings(&source_recording_path, &candidate_recording_path),
            RunnerCommand::Ping { seq } => Ok(json!({ "pong": true, "seq": seq })),
        };

        Self::response_from_result(correlation_id, result)
    }

    fn response_from_result(correlation_id: String, result: HandlerResult) -> RunnerResponse {
        match result {
            Ok(result) => RunnerResponse {
                version: IPC_VERSION,
                correlation_id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => RunnerResponse {
                version: IPC_VERSION,
                correlation_id: error.correlation_id.clone(),
                ok: false,
                result: None,
                error: Some(*error),
            },
        }
    }

    fn validate(&self, path: &str) -> HandlerResult {
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

    fn create_run(&mut self, path: &str) -> HandlerResult {
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
        self.emit(RunnerEvent::SimulationStateChanged {
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

    async fn create_live_run(&mut self, path: &str, timeout_ms: u64) -> HandlerResult {
        let scenario =
            load_scenario(path).map_err(|error| Box::new(Self::simulation_error(error, None)))?;
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
        let run_id = format!("live-run-{}", scenario.id);
        self.simulation = Some(Simulation::new(run_id.clone(), scenario.clone()));
        self.recording = Some(Recording::new(run_id.clone(), &scenario));
        self.server = LocalMcpServer::default();
        self.agent = RuleAgent::default();
        self.live_driver = HumanAgentDriver::new();
        self.live_backend = Some(backend);
        self.plugin_host = PluginHost::default();
        self.plugin_executors.clear();
        self.recording_queue = RecordingQueue::new(256, RecordingQueuePolicy::FailRun);
        self.emit(RunnerEvent::SimulationStateChanged {
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

    fn start(&mut self) -> HandlerResult {
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
        self.emit(RunnerEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Running,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({ "runId": run_id, "status": RunStatus::Running }))
    }

    fn resume_run(&mut self, scenario_path: &str, run_id: &str) -> HandlerResult {
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
                self.emit(RunnerEvent::SimulationEvent { cursor: 0, event });
            }
            for trace in step.tool_calls {
                self.emit(RunnerEvent::SimulationToolCall { cursor: 0, trace });
            }
            for result in step.action_results {
                self.emit(RunnerEvent::SimulationActionResult { cursor: 0, result });
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

    fn pause(&mut self) -> HandlerResult {
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
        self.emit(RunnerEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Paused,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({ "runId": run_id, "status": RunStatus::Paused }))
    }

    fn step(&mut self) -> HandlerResult {
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
            self.emit(RunnerEvent::SimulationError {
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
            self.emit(RunnerEvent::SimulationEvent { cursor: 0, event });
        }
        for trace in step.tool_calls {
            self.emit(RunnerEvent::SimulationToolCall { cursor: 0, trace });
        }
        for result in step.action_results {
            self.emit(RunnerEvent::SimulationActionResult { cursor: 0, result });
        }
        for failure in &step.plugin_failures {
            self.emit(RunnerEvent::SimulationPluginFailure {
                cursor: 0,
                failure: failure.clone(),
            });
        }
        if !plugin_failures.is_empty()
            && matches!(plugin_status, RunStatus::Paused | RunStatus::Failed)
        {
            self.emit(RunnerEvent::SimulationStateChanged {
                cursor: 0,
                state: plugin_status,
                run_id: Some(simulation.run_id().to_string()),
            });
        }
        if let Some(recording) = self.recording.as_ref() {
            let evaluation = cockpit_evaluation::evaluate_scenario(recording, &simulation.scenario);
            self.emit(RunnerEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation: serde_json::to_value(evaluation).unwrap_or(Value::Null),
            });
        }
        if deadline_reached(
            simulation.status,
            tick,
            simulation.scenario.shutdown_deadline_ticks,
        ) {
            simulation.status = RunStatus::Completed;
            self.emit(RunnerEvent::SimulationStateChanged {
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

    async fn step_live(&mut self) -> HandlerResult {
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
            .step_with_backend(&mut simulation, &mut backend)
            .await;
        let cancelled = self.live_turn_control.is_cancelled();
        self.live_turn_control.finish();
        self.live_backend = Some(backend);

        if cancelled {
            simulation.stop();
            let run_id = simulation.run_id().to_string();
            self.emit(RunnerEvent::SimulationStateChanged {
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
                self.emit_execution_failure_evaluation(&simulation.scenario, error.to_string());
                let ipc_error = IpcError {
                    code: "LIVE_BACKEND_TURN_FAILED".to_string(),
                    message: error.to_string(),
                    details: None,
                    run_id: Some(simulation.run_id().to_string()),
                    tick: Some(simulation.snapshot.tick),
                    correlation_id: "live-backend".to_string(),
                };
                self.emit(RunnerEvent::SimulationError {
                    cursor: 0,
                    error: ipc_error.clone(),
                });
                self.emit(RunnerEvent::SimulationStateChanged {
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
        if let Some(recording) = self.recording.as_mut() {
            recording.push(step.clone());
            recording.push_human_turns(human_turns.clone());
        }
        self.emit_persist_recording_failure(&simulation, tick);
        self.emit(Self::tick_committed_event(&snapshot));
        for evidence in &human_turns {
            self.emit(RunnerEvent::SimulationHumanTurn {
                cursor: 0,
                tick,
                backend: backend_label.to_string(),
                evidence: evidence.clone(),
            });
        }
        for event in step.events {
            self.emit(RunnerEvent::SimulationEvent { cursor: 0, event });
        }
        for trace in step.tool_calls {
            self.emit(RunnerEvent::SimulationToolCall { cursor: 0, trace });
        }
        for result in step.action_results {
            self.emit(RunnerEvent::SimulationActionResult { cursor: 0, result });
        }
        if let Some(recording) = self.recording.as_ref() {
            let evaluation = cockpit_evaluation::evaluate_scenario(recording, &simulation.scenario);
            self.emit(RunnerEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation: serde_json::to_value(evaluation).unwrap_or(Value::Null),
            });
        }
        if deadline_reached(
            simulation.status,
            tick,
            simulation.scenario.shutdown_deadline_ticks,
        ) {
            simulation.status = RunStatus::Completed;
            self.emit(RunnerEvent::SimulationStateChanged {
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

    fn emit_execution_failure_evaluation(&mut self, scenario: &SimulationScenario, error: String) {
        if let Some(recording) = self.recording.as_ref() {
            let evaluation = cockpit_evaluation::mark_execution_failed(
                cockpit_evaluation::evaluate_scenario(recording, scenario),
                error,
            );
            self.emit(RunnerEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation: serde_json::to_value(evaluation).unwrap_or(Value::Null),
            });
        }
    }

    fn stop(&mut self) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        simulation.stop();
        let run_id = simulation.run_id().to_string();
        self.simulation = Some(simulation);
        self.emit(RunnerEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Stopped,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({ "runId": run_id, "status": RunStatus::Stopped }))
    }

    fn approve_action(&mut self, request_id: &str) -> HandlerResult {
        let mut simulation = self
            .simulation
            .take()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let result = self.server.approve_action(&mut simulation, request_id);
        self.simulation = Some(simulation);
        let result = result.map_err(|error| Box::new(Self::tool_error(error)))?;
        self.emit(RunnerEvent::SimulationActionResult {
            cursor: 0,
            result: result.clone(),
        });
        serde_json::to_value(result)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
    }

    fn reject_action(&mut self, request_id: &str, reason: Option<&str>) -> HandlerResult {
        let simulation = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let result = self
            .server
            .reject_action(simulation, request_id, false)
            .map_err(|error| Box::new(Self::tool_error(error)))?;
        self.emit(RunnerEvent::SimulationActionResult {
            cursor: 0,
            result: result.clone(),
        });
        Ok(json!({
            "result": result,
            "reason": reason
        }))
    }

    fn cancel_agent_turn(&mut self) -> HandlerResult {
        let simulation = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        let results = self.server.cancel_pending_actions(simulation);
        for result in &results {
            self.emit(RunnerEvent::SimulationActionResult {
                cursor: 0,
                result: result.clone(),
            });
        }
        Ok(json!({ "cancelled": true, "count": results.len() }))
    }

    fn set_approval_required(&mut self, required: bool) -> HandlerResult {
        self.server.set_approval_required(required);
        Ok(json!({ "approvalRequired": required }))
    }

    fn persist_recording(&mut self) -> HandlerResult {
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
    fn emit_persist_recording_failure(&mut self, simulation: &Simulation, tick: u64) {
        if let Err(error) = self.persist_recording() {
            self.emit(RunnerEvent::SimulationError {
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

    fn snapshot(&self) -> HandlerResult {
        let simulation = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?;
        serde_json::to_value(&simulation.snapshot)
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
    }

    fn start_replay(&mut self, scenario_path: &str, recording_path: &str) -> HandlerResult {
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
        self.emit(RunnerEvent::SimulationStateChanged {
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
                self.emit(RunnerEvent::SimulationEvent { cursor: 0, event });
            }
            for trace in step.tool_calls {
                self.emit(RunnerEvent::SimulationToolCall { cursor: 0, trace });
            }
            for result in step.action_results {
                self.emit(RunnerEvent::SimulationActionResult { cursor: 0, result });
            }
        }
        simulation.status = RunStatus::Completed;
        self.simulation = Some(simulation);
        self.recording = Some(replay.clone());
        self.emit(RunnerEvent::SimulationStateChanged {
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

    fn diff_recordings(
        &self,
        source_recording_path: &str,
        candidate_recording_path: &str,
    ) -> HandlerResult {
        let source = read_recording(source_recording_path)?;
        let candidate = read_recording(candidate_recording_path)?;
        serde_json::to_value(diff_recordings(&source, &candidate))
            .map_err(|error| Box::new(Self::serialization_error(error.to_string())))
    }

    fn events_after(&self, cursor: Option<u64>) -> Vec<RunnerEvent> {
        let cursor = cursor.unwrap_or(0);
        self.events
            .iter()
            .filter(|event| event.cursor() > cursor)
            .cloned()
            .collect()
    }

    fn tick_committed_event(snapshot: &WorldSnapshot) -> RunnerEvent {
        RunnerEvent::SimulationTickCommitted {
            cursor: 0,
            run_id: snapshot.run_id.clone(),
            tick: snapshot.tick,
            sim_time_ms: snapshot.sim_time_ms,
            version: snapshot.version,
        }
    }

    fn cursor_reset_required(&self, cursor: Option<u64>) -> bool {
        let Some(cursor) = cursor else {
            return false;
        };
        let Some(first) = self.events.first().map(RunnerEvent::cursor) else {
            return false;
        };
        cursor.saturating_add(1) < first
    }

    fn run_plugins(&mut self, simulation: &Simulation) -> (Vec<StateDiff>, Vec<PluginFailure>) {
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

    fn emit_plugin_failure(&mut self, failure: &PluginFailure) {
        self.emit(RunnerEvent::SimulationPluginFailure {
            cursor: 0,
            failure: plugin_failure_record(failure),
        });
    }

    fn emit(&mut self, event: RunnerEvent) {
        self.next_cursor += 1;
        let cursor = self.next_cursor;
        let event = match event {
            RunnerEvent::SimulationStateChanged { state, run_id, .. } => {
                RunnerEvent::SimulationStateChanged {
                    cursor,
                    state,
                    run_id,
                }
            }
            RunnerEvent::SimulationTickCommitted {
                run_id,
                tick,
                sim_time_ms,
                version,
                ..
            } => RunnerEvent::SimulationTickCommitted {
                cursor,
                run_id,
                tick,
                sim_time_ms,
                version,
            },
            RunnerEvent::SimulationEvent { event, .. } => {
                RunnerEvent::SimulationEvent { cursor, event }
            }
            RunnerEvent::SimulationToolCall { trace, .. } => {
                RunnerEvent::SimulationToolCall { cursor, trace }
            }
            RunnerEvent::SimulationHumanTurn {
                tick,
                backend,
                evidence,
                ..
            } => RunnerEvent::SimulationHumanTurn {
                cursor,
                tick,
                backend,
                evidence,
            },
            RunnerEvent::SimulationActionResult { result, .. } => {
                RunnerEvent::SimulationActionResult { cursor, result }
            }
            RunnerEvent::SimulationPluginFailure { failure, .. } => {
                RunnerEvent::SimulationPluginFailure { cursor, failure }
            }
            RunnerEvent::SimulationEvaluationUpdated { evaluation, .. } => {
                RunnerEvent::SimulationEvaluationUpdated { cursor, evaluation }
            }
            RunnerEvent::SimulationError { error, .. } => {
                RunnerEvent::SimulationError { cursor, error }
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
    ) -> RunnerResponse {
        RunnerResponse {
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

    fn no_run_error() -> IpcError {
        IpcError {
            code: "RUN_NOT_CREATED".to_string(),
            message: "create a simulation run first".to_string(),
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "runner".to_string(),
        }
    }

    fn simulation_error(error: SimulationError, simulation: Option<&Simulation>) -> IpcError {
        IpcError {
            code: "SIMULATION_ERROR".to_string(),
            message: error.to_string(),
            details: None,
            run_id: simulation.map(|value| value.run_id().to_string()),
            tick: simulation.map(|value| value.snapshot.tick),
            correlation_id: "runner".to_string(),
        }
    }

    fn serialization_error(message: String) -> IpcError {
        IpcError {
            code: "SERIALIZATION_ERROR".to_string(),
            message,
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "runner".to_string(),
        }
    }

    fn tool_error(error: cockpit_agent_runtime::ToolError) -> IpcError {
        IpcError {
            code: error.code,
            message: error.message,
            details: None,
            run_id: None,
            tick: None,
            correlation_id: "runner".to_string(),
        }
    }
}

fn deadline_reached(status: RunStatus, tick: u64, deadline_tick: u64) -> bool {
    matches!(status, RunStatus::Running) && tick >= deadline_tick
}

fn plugin_failure_record(failure: &PluginFailure) -> PluginFailureRecord {
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
    use super::{LiveTurnControl, RunnerEvent, RunnerHandler, deadline_reached};
    use cockpit_recording::run_rule_agent_recording;
    use cockpit_scenario::load_scenario;
    use cockpit_simulation_core::clock::RunStatus;

    #[test]
    fn deadline_completion_only_applies_to_running_simulations() {
        assert!(deadline_reached(RunStatus::Running, 16, 16));
        assert!(deadline_reached(RunStatus::Running, 17, 16));
        assert!(!deadline_reached(RunStatus::Running, 15, 16));
        assert!(!deadline_reached(RunStatus::Paused, 16, 16));
        assert!(!deadline_reached(RunStatus::Failed, 16, 16));
    }

    #[test]
    fn live_turn_control_cancels_without_runner_state_access() {
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
            scenario.shutdown_deadline_ticks + 1,
        )
        .expect("recording");
        let mut handler = RunnerHandler::new("session");
        handler.recording = Some(recording);
        handler.emit_execution_failure_evaluation(&scenario, "backend timeout".to_string());

        let RunnerEvent::SimulationEvaluationUpdated { evaluation, .. } =
            handler.events.last().expect("evaluation event")
        else {
            panic!("last event must be an evaluation update");
        };
        assert_eq!(evaluation["passed"], false);
        assert_eq!(evaluation["executionPassed"], false);
        assert_eq!(evaluation["executionError"], "backend timeout");
    }
}
