use cockpit_agent::GoalStatus;
use cockpit_agent::HumanTurnEvidence;
use cockpit_world::{
    DynamicEntity, PluginFailureRecord,
    action::ActionResult,
    clock::RunStatus,
    event::{EventEnvelope, ToolCallTrace},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const IPC_VERSION: u16 = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(
    clippy::large_enum_variant,
    reason = "Boxing variants would alter the versioned JSON IPC request API."
)]
pub enum SimulatorCommand {
    #[serde(rename = "ValidateScenario")]
    ValidateScenario { path: String },
    #[serde(rename = "CreateSimulationRun")]
    CreateSimulationRun { path: String },
    #[serde(rename = "CreateLiveSimulationRun")]
    CreateLiveSimulationRun { path: String, timeout_ms: u64 },
    #[serde(rename = "ResumeLiveSimulation")]
    ResumeLiveSimulation {
        scenario_path: String,
        run_id: String,
        timeout_ms: u64,
    },
    #[serde(rename = "ResumeSimulation")]
    ResumeSimulation {
        scenario_path: String,
        run_id: String,
    },
    #[serde(rename = "SpawnEntity")]
    SpawnEntity { entity: DynamicEntity },
    #[serde(rename = "RemoveEntity")]
    RemoveEntity { entity_id: String },
    #[serde(rename = "AddAgentGoal")]
    AddAgentGoal {
        agent_id: String,
        description: String,
        priority: i32,
    },
    #[serde(rename = "SetAgentGoalStatus")]
    SetAgentGoalStatus {
        agent_id: String,
        goal_id: String,
        status: GoalStatus,
    },
    #[serde(rename = "WaitAgentUntil")]
    WaitAgentUntil { agent_id: String, wake_tick: u64 },
    #[serde(rename = "GetOpenWorldRuntime")]
    GetOpenWorldRuntime,
    #[serde(rename = "CheckpointOpenWorld")]
    CheckpointOpenWorld,
    #[serde(rename = "StartSimulation")]
    StartSimulation,
    #[serde(rename = "PauseSimulation")]
    PauseSimulation,
    #[serde(rename = "StepSimulation")]
    StepSimulation,
    #[serde(rename = "StepLiveSimulation")]
    StepLiveSimulation,
    #[serde(rename = "CancelLiveTurn")]
    CancelLiveTurn,
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
    #[serde(rename = "SetApprovalRequired")]
    SetApprovalRequired { required: bool },
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
    #[serde(rename = "DiffRecordings")]
    DiffRecordings {
        source_recording_path: String,
        candidate_recording_path: String,
    },
    /// Lightweight liveness probe, used by the desktop client's heartbeat
    /// loop to detect a wedged or crashed simulator process without invoking
    /// any simulation logic. Answered with `{"pong": true, "seq": <seq>}`.
    #[serde(rename = "Ping")]
    Ping { seq: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatorRequest {
    pub version: u16,
    pub session_token: String,
    pub correlation_id: String,
    pub command: SimulatorCommand,
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
pub struct SimulatorResponse {
    pub version: u16,
    pub correlation_id: String,
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<IpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum SimulatorEvent {
    #[serde(rename = "SimulationStateChanged")]
    SimulationStateChanged {
        cursor: u64,
        state: RunStatus,
        run_id: Option<String>,
    },
    #[serde(rename = "SimulationTickCommitted")]
    SimulationTickCommitted {
        cursor: u64,
        run_id: String,
        tick: u64,
        sim_time_ms: u64,
        version: u64,
    },
    #[serde(rename = "SimulationEvent")]
    SimulationEvent { cursor: u64, event: EventEnvelope },
    #[serde(rename = "SimulationToolCall")]
    SimulationToolCall { cursor: u64, trace: ToolCallTrace },
    #[serde(rename = "SimulationHumanTurn")]
    SimulationHumanTurn {
        cursor: u64,
        tick: u64,
        backend: String,
        evidence: HumanTurnEvidence,
    },
    #[serde(rename = "SimulationActionResult")]
    SimulationActionResult { cursor: u64, result: ActionResult },
    #[serde(rename = "SimulationPluginFailure")]
    SimulationPluginFailure {
        cursor: u64,
        failure: PluginFailureRecord,
    },
    #[serde(rename = "SimulationEvaluationUpdated")]
    SimulationEvaluationUpdated { cursor: u64, evaluation: Value },
    #[serde(rename = "SimulationError")]
    SimulationError { cursor: u64, error: IpcError },
}

impl SimulatorEvent {
    pub fn cursor(&self) -> u64 {
        match self {
            Self::SimulationStateChanged { cursor, .. }
            | Self::SimulationTickCommitted { cursor, .. }
            | Self::SimulationEvent { cursor, .. }
            | Self::SimulationToolCall { cursor, .. }
            | Self::SimulationHumanTurn { cursor, .. }
            | Self::SimulationActionResult { cursor, .. }
            | Self::SimulationPluginFailure { cursor, .. }
            | Self::SimulationEvaluationUpdated { cursor, .. }
            | Self::SimulationError { cursor, .. } => *cursor,
        }
    }
}
