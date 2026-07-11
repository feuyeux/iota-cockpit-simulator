use cockpit_simulation_core::{
    action::ActionResult,
    clock::RunStatus,
    event::{EventEnvelope, ToolCallTrace},
    world::WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const IPC_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RunnerCommand {
    #[serde(rename = "ValidateScenario")]
    ValidateScenario { path: String },
    #[serde(rename = "CreateSimulationRun")]
    CreateSimulationRun { path: String },
    #[serde(rename = "StartSimulation")]
    StartSimulation,
    #[serde(rename = "PauseSimulation")]
    PauseSimulation,
    #[serde(rename = "StepSimulation")]
    StepSimulation,
    #[serde(rename = "StopSimulation")]
    StopSimulation,
    #[serde(rename = "ApproveAction")]
    ApproveAction { request_id: String },
    #[serde(rename = "RejectAction")]
    RejectAction {
        request_id: String,
        reason: Option<String>,
    },
    #[serde(rename = "CancelAgentTurn")]
    CancelAgentTurn,
    #[serde(rename = "GetSimulationSnapshot")]
    GetSimulationSnapshot,
    #[serde(rename = "GetSimulationEvents")]
    GetSimulationEvents { cursor: Option<u64> },
    #[serde(rename = "GetAgentTrace")]
    GetAgentTrace,
    #[serde(rename = "StartReplay")]
    StartReplay {
        scenario_path: String,
        recording_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerRequest {
    pub version: u16,
    pub session_token: String,
    pub correlation_id: String,
    pub command: RunnerCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
    pub run_id: Option<String>,
    pub tick: Option<u64>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerResponse {
    pub version: u16,
    pub correlation_id: String,
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<IpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum RunnerEvent {
    #[serde(rename = "SimulationStateChanged")]
    SimulationStateChanged {
        cursor: u64,
        state: RunStatus,
        run_id: Option<String>,
    },
    #[serde(rename = "SimulationTickCommitted")]
    SimulationTickCommitted {
        cursor: u64,
        snapshot: WorldSnapshot,
    },
    #[serde(rename = "SimulationEvent")]
    SimulationEvent { cursor: u64, event: EventEnvelope },
    #[serde(rename = "SimulationToolCall")]
    SimulationToolCall { cursor: u64, trace: ToolCallTrace },
    #[serde(rename = "SimulationActionResult")]
    SimulationActionResult { cursor: u64, result: ActionResult },
    #[serde(rename = "SimulationEvaluationUpdated")]
    SimulationEvaluationUpdated { cursor: u64, evaluation: Value },
    #[serde(rename = "SimulationError")]
    SimulationError { cursor: u64, error: IpcError },
}

impl RunnerEvent {
    pub fn cursor(&self) -> u64 {
        match self {
            Self::SimulationStateChanged { cursor, .. }
            | Self::SimulationTickCommitted { cursor, .. }
            | Self::SimulationEvent { cursor, .. }
            | Self::SimulationToolCall { cursor, .. }
            | Self::SimulationActionResult { cursor, .. }
            | Self::SimulationEvaluationUpdated { cursor, .. }
            | Self::SimulationError { cursor, .. } => *cursor,
        }
    }
}
