use std::sync::Mutex;

use cockpit_runner::ipc::{
    RunnerHandler,
    proto::{IPC_VERSION, RunnerCommand, RunnerEvent, RunnerRequest},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct RunnerState {
    handler: Mutex<RunnerHandler>,
    token: String,
    sequence: Mutex<u64>,
}

impl RunnerState {
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        Self {
            handler: Mutex::new(RunnerHandler::new(token.clone())),
            token,
            sequence: Mutex::new(0),
        }
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
        let response = self
            .handler
            .lock()
            .map_err(|_| "runner lock poisoned".to_string())?
            .dispatch(request);
        if response.ok {
            Ok(response.result.unwrap_or(Value::Null))
        } else {
            Err(response
                .error
                .map(|error| format!("{}: {}", error.code, error.message))
                .unwrap_or_else(|| "runner command failed".to_string()))
        }
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
pub fn connect_runner() -> Result<(), String> {
    Ok(())
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
pub fn get_simulation_events(
    state: tauri::State<'_, RunnerState>,
    cursor: Option<u64>,
) -> Result<Vec<RunnerEvent>, String> {
    let result = state.dispatch(RunnerCommand::GetSimulationEvents { cursor })?;
    serde_json::from_value(
        result
            .get("events")
            .cloned()
            .unwrap_or(Value::Array(Vec::new())),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_simulation_snapshot(state: tauri::State<'_, RunnerState>) -> Result<Value, String> {
    state.dispatch(RunnerCommand::GetSimulationSnapshot)
}
