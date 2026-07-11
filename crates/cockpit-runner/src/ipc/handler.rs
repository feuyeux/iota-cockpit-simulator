use std::{fs, path::Path};

use cockpit_agent_runtime::{LocalMcpServer, RuleAgent};
use cockpit_evaluation::evaluate_smoke_shutdown;
use cockpit_recording::{Recording, replay_recording};
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::{Simulation, SimulationError, clock::RunStatus};
use serde_json::{Value, json};

use super::proto::{
    IPC_VERSION, IpcError, RunnerCommand, RunnerEvent, RunnerRequest, RunnerResponse,
};

type HandlerResult = Result<Value, Box<IpcError>>;

pub struct RunnerHandler {
    session_token: String,
    simulation: Option<Simulation>,
    recording: Option<Recording>,
    server: LocalMcpServer,
    agent: RuleAgent,
    events: Vec<RunnerEvent>,
    next_cursor: u64,
}

impl RunnerHandler {
    pub fn new(session_token: impl Into<String>) -> Self {
        Self {
            session_token: session_token.into(),
            simulation: None,
            recording: None,
            server: LocalMcpServer::default(),
            agent: RuleAgent::default(),
            events: Vec::new(),
            next_cursor: 0,
        }
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
            RunnerCommand::StartSimulation => self.start(),
            RunnerCommand::PauseSimulation => self.pause(),
            RunnerCommand::StepSimulation => self.step(),
            RunnerCommand::StopSimulation => self.stop(),
            RunnerCommand::ApproveAction { request_id } => self.approve_action(&request_id),
            RunnerCommand::RejectAction { request_id, reason } => {
                self.reject_action(&request_id, reason.as_deref())
            }
            RunnerCommand::CancelAgentTurn => self.cancel_agent_turn(),
            RunnerCommand::GetSimulationSnapshot => self.snapshot(),
            RunnerCommand::GetSimulationEvents { cursor } => Ok(json!({
                "events": self.events_after(cursor),
                "nextCursor": self.next_cursor
            })),
            RunnerCommand::GetAgentTrace => Ok(json!({
                "events": self
                    .events
                    .iter()
                    .filter(|event| matches!(event, RunnerEvent::SimulationToolCall { .. }))
                    .collect::<Vec<_>>()
            })),
            RunnerCommand::StartReplay {
                scenario_path,
                recording_path,
            } => self.start_replay(&scenario_path, &recording_path),
        };

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
        self.emit(RunnerEvent::SimulationStateChanged {
            cursor: 0,
            state: RunStatus::Ready,
            run_id: Some(run_id.clone()),
        });
        Ok(json!({
            "runId": run_id,
            "status": RunStatus::Ready,
            "scenarioHash": scenario.scenario_hash
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
        let result = self.agent.step(&mut simulation, &mut self.server);
        let step = match result {
            Ok(step) => step,
            Err(error) => {
                let ipc_error = Self::simulation_error(error, Some(&simulation));
                self.simulation = Some(simulation);
                return Err(Box::new(ipc_error));
            }
        };
        let tick = step.tick;
        let snapshot = simulation.snapshot.clone();
        let snapshot_hash = step.snapshot_hash.clone();
        if let Some(recording) = self.recording.as_mut() {
            recording.push(step.clone());
        }
        self.emit(RunnerEvent::SimulationTickCommitted {
            cursor: 0,
            snapshot,
        });
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
            let evaluation =
                evaluate_smoke_shutdown(recording, simulation.scenario.shutdown_deadline_ticks);
            self.emit(RunnerEvent::SimulationEvaluationUpdated {
                cursor: 0,
                evaluation: serde_json::to_value(evaluation).unwrap_or(Value::Null),
            });
        }
        let run_id = simulation.run_id().to_string();
        self.simulation = Some(simulation);
        Ok(json!({
            "runId": run_id,
            "tick": tick,
            "snapshotHash": snapshot_hash
        }))
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
        Ok(json!({
            "result": result,
            "reason": reason
        }))
    }

    fn cancel_agent_turn(&mut self) -> HandlerResult {
        Ok(json!({ "cancelled": true }))
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
        let replay = replay_recording("replay-run", scenario, &recording)
            .map_err(|error| Box::new(Self::simulation_error(error, None)))?;
        Ok(json!({
            "runId": replay.run_id,
            "ticks": replay.ticks.len(),
            "finalSnapshotHash": replay.final_snapshot_hash()
        }))
    }

    fn events_after(&self, cursor: Option<u64>) -> Vec<RunnerEvent> {
        let cursor = cursor.unwrap_or(0);
        self.events
            .iter()
            .filter(|event| event.cursor() > cursor)
            .cloned()
            .collect()
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
            RunnerEvent::SimulationTickCommitted { snapshot, .. } => {
                RunnerEvent::SimulationTickCommitted { cursor, snapshot }
            }
            RunnerEvent::SimulationEvent { event, .. } => {
                RunnerEvent::SimulationEvent { cursor, event }
            }
            RunnerEvent::SimulationToolCall { trace, .. } => {
                RunnerEvent::SimulationToolCall { cursor, trace }
            }
            RunnerEvent::SimulationActionResult { result, .. } => {
                RunnerEvent::SimulationActionResult { cursor, result }
            }
            RunnerEvent::SimulationEvaluationUpdated { evaluation, .. } => {
                RunnerEvent::SimulationEvaluationUpdated { cursor, evaluation }
            }
            RunnerEvent::SimulationError { error, .. } => {
                RunnerEvent::SimulationError { cursor, error }
            }
        };
        self.events.push(event);
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
