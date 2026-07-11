use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    process::{Child, Command},
    sync::Mutex,
    time::Duration,
};

use cockpit_runner::ipc::{
    RunnerHandler,
    proto::{IPC_VERSION, RunnerCommand, RunnerEvent, RunnerRequest, RunnerResponse},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

enum RunnerTransport {
    Embedded(Box<RunnerHandler>),
    Process { child: Child, address: SocketAddr },
}

impl Drop for RunnerTransport {
    fn drop(&mut self) {
        if let Self::Process { child, .. } = self {
            let _ = child.kill();
        }
    }
}

pub struct RunnerState {
    transport: Mutex<RunnerTransport>,
    token: String,
    sequence: Mutex<u64>,
}

impl RunnerState {
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        Self {
            transport: Mutex::new(RunnerTransport::Embedded(Box::new(RunnerHandler::new(
                token.clone(),
            )))),
            token,
            sequence: Mutex::new(0),
        }
    }

    pub fn connect(&self) -> Result<String, String> {
        let Some(binary) = std::env::var_os("COCKPIT_RUNNER_BIN") else {
            return Ok("embedded".to_string());
        };
        let address: SocketAddr = "127.0.0.1:47701"
            .parse()
            .map_err(|error| format!("invalid runner address: {error}"))?;
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| "runner transport lock poisoned".to_string())?;
        if let RunnerTransport::Process { address, .. } = &*transport
            && TcpStream::connect_timeout(address, Duration::from_millis(20)).is_ok()
        {
            return Ok("process".to_string());
        }
        *transport = Self::spawn_process(binary, address, &self.token)?;
        Ok("process".to_string())
    }

    fn spawn_process(
        binary: std::ffi::OsString,
        address: SocketAddr,
        token: &str,
    ) -> Result<RunnerTransport, String> {
        let child = Command::new(binary)
            .args([
                "serve",
                "--bind",
                &address.to_string(),
                "--session-token",
                token,
            ])
            .spawn()
            .map_err(|error| format!("failed to start cockpit-runner: {error}"))?;
        let connected = (0..50)
            .any(|_| TcpStream::connect_timeout(&address, Duration::from_millis(20)).is_ok());
        if !connected {
            let mut child = child;
            let _ = child.kill();
            return Err("cockpit-runner did not accept loopback connections".to_string());
        }
        Ok(RunnerTransport::Process { child, address })
    }

    fn dispatch(&self, command: RunnerCommand) -> Result<Value, String> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = RunnerRequest {
            version: IPC_VERSION,
            session_token: self.token.clone(),
            correlation_id: format!("desktop-{}", *sequence),
            command,
        };
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| "runner transport lock poisoned".to_string())?;
        let response = match &mut *transport {
            RunnerTransport::Embedded(handler) => return response_value(handler.dispatch(request)),
            RunnerTransport::Process { address, .. } => request_process(*address, &request),
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let Some(binary) = std::env::var_os("COCKPIT_RUNNER_BIN") else {
                    return Err(error);
                };
                let address: SocketAddr = "127.0.0.1:47701"
                    .parse()
                    .map_err(|parse_error| format!("invalid runner address: {parse_error}"))?;
                *transport = Self::spawn_process(binary, address, &self.token)?;
                match &mut *transport {
                    RunnerTransport::Process { address, .. } => {
                        request_process(*address, &request)?
                    }
                    RunnerTransport::Embedded(_) => unreachable!(),
                }
            }
        };
        response_value(response)
    }
}

fn request_process(address: SocketAddr, request: &RunnerRequest) -> Result<RunnerResponse, String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))
        .map_err(|error| format!("runner disconnected: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(2_000)))
        .map_err(|error| error.to_string())?;
    let mut encoded = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    stream
        .write_all(&encoded)
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| format!("runner response failed: {error}"))?;
    serde_json::from_str(&line).map_err(|error| format!("runner response invalid: {error}"))
}

fn response_value(response: RunnerResponse) -> Result<Value, String> {
    if response.ok {
        Ok(response.result.unwrap_or(Value::Null))
    } else {
        Err(response
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "runner command failed".to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioSummary {
    pub id: String,
    pub path: String,
    pub schema_version: u32,
    pub scenario_hash: String,
    pub seed: u64,
    pub agent_id: String,
}

#[tauri::command]
pub fn connect_runner(state: tauri::State<'_, RunnerState>) -> Result<String, String> {
    state.connect()
}

#[tauri::command]
pub fn validate_scenario(
    state: tauri::State<'_, RunnerState>,
    path: String,
) -> Result<ScenarioSummary, String> {
    serde_json::from_value(state.dispatch(RunnerCommand::ValidateScenario { path })?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_simulation_run(
    state: tauri::State<'_, RunnerState>,
    path: String,
) -> Result<String, String> {
    state
        .dispatch(RunnerCommand::CreateSimulationRun { path })?
        .get("runId")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "runner did not return runId".to_string())
}

#[tauri::command]
pub fn start_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::StartSimulation).map(|_| ())
}

#[tauri::command]
pub fn pause_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::PauseSimulation).map(|_| ())
}

#[tauri::command]
pub fn step_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::StepSimulation).map(|_| ())
}

#[tauri::command]
pub fn stop_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::StopSimulation).map(|_| ())
}

#[tauri::command]
pub fn resume_simulation(
    state: tauri::State<'_, RunnerState>,
    scenario_path: String,
    run_id: String,
) -> Result<(), String> {
    state
        .dispatch(RunnerCommand::ResumeSimulation {
            scenario_path,
            run_id,
        })
        .map(|_| ())
}

#[tauri::command]
pub fn approve_action(
    state: tauri::State<'_, RunnerState>,
    request_id: String,
) -> Result<Value, String> {
    state.dispatch(RunnerCommand::ApproveAction { request_id })
}

#[tauri::command]
pub fn reject_action(
    state: tauri::State<'_, RunnerState>,
    request_id: String,
    reason: Option<String>,
) -> Result<Value, String> {
    state.dispatch(RunnerCommand::RejectAction { request_id, reason })
}

#[tauri::command]
pub fn cancel_agent_turn(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::CancelAgentTurn).map(|_| ())
}

#[tauri::command]
pub fn set_approval_required(
    state: tauri::State<'_, RunnerState>,
    required: bool,
) -> Result<(), String> {
    state
        .dispatch(RunnerCommand::SetApprovalRequired { required })
        .map(|_| ())
}

#[tauri::command]
pub fn start_replay(
    state: tauri::State<'_, RunnerState>,
    scenario_path: String,
    recording_path: String,
) -> Result<Value, String> {
    state.dispatch(RunnerCommand::StartReplay {
        scenario_path,
        recording_path,
    })
}

#[tauri::command]
pub fn diff_recordings(
    state: tauri::State<'_, RunnerState>,
    source_recording_path: String,
    candidate_recording_path: String,
) -> Result<Value, String> {
    state.dispatch(RunnerCommand::DiffRecordings {
        source_recording_path,
        candidate_recording_path,
    })
}

#[tauri::command]
pub fn get_simulation_events(
    state: tauri::State<'_, RunnerState>,
    cursor: Option<u64>,
) -> Result<SimulationEventBatch, String> {
    let result = state.dispatch(RunnerCommand::GetSimulationEvents { cursor })?;
    serde_json::from_value(result).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationEventBatch {
    pub events: Vec<RunnerEvent>,
    pub next_cursor: u64,
    pub first_available_cursor: u64,
    pub reset_required: bool,
}

#[tauri::command]
pub fn get_simulation_snapshot(state: tauri::State<'_, RunnerState>) -> Result<Value, String> {
    state.dispatch(RunnerCommand::GetSimulationSnapshot)
}
