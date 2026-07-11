use serde::{Deserialize, Serialize};

use crate::{id::AgentId, sensor::Observation};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Command {
    EngineShutdown,
    AlarmActivate,
}

impl Command {
    pub fn capability_name(&self) -> &'static str {
        match self {
            Self::EngineShutdown => "engine.shutdown",
            Self::AlarmActivate => "alarm.activate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    CapabilityDenied,
    DeviceUnpowered,
    PreconditionFailed,
    #[serde(rename = "STATE_VERSION_CONFLICT")]
    VersionMismatch,
    ActionExpired,
    ActionConflict,
    UnknownTarget,
    ApprovalDenied,
    ActionCancelled,
}

impl ErrorCode {
    pub fn stable_code(&self) -> &'static str {
        match self {
            Self::CapabilityDenied => "CAPABILITY_DENIED",
            Self::DeviceUnpowered => "DEVICE_UNPOWERED",
            Self::PreconditionFailed => "PRECONDITION_FAILED",
            Self::VersionMismatch => "STATE_VERSION_CONFLICT",
            Self::ActionExpired => "ACTION_EXPIRED",
            Self::ActionConflict => "ACTION_CONFLICT",
            Self::UnknownTarget => "UNKNOWN_TARGET",
            Self::ApprovalDenied => "APPROVAL_DENIED",
            Self::ActionCancelled => "ACTION_CANCELLED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionStatus {
    Applied,
    Rejected,
    Superseded,
    PendingApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    pub request_id: String,
    pub agent_id: AgentId,
    pub target: String,
    pub command: Command,
    pub expected_state_version: u64,
    pub expires_at_tick: u64,
    #[serde(default)]
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub request: ActionRequest,
    pub status: ActionStatus,
    pub error_code: Option<ErrorCode>,
    pub run_id: String,
    pub tick: u64,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGrant {
    pub agent_id: AgentId,
    pub capabilities: Vec<String>,
}

impl AgentGrant {
    pub fn allows(&self, agent_id: &str, command: &Command) -> bool {
        self.agent_id == agent_id
            && self
                .capabilities
                .iter()
                .any(|capability| capability == command.capability_name())
    }
}

#[derive(Debug, Default)]
pub struct ScriptedAgent {
    action_sent: bool,
}

impl ScriptedAgent {
    pub fn next_actions(
        &mut self,
        observation: &Observation,
        state_version: u64,
    ) -> Vec<ActionRequest> {
        if self.action_sent
            || !observation
                .alerts
                .iter()
                .any(|alert| alert == "SmokeDetected")
        {
            return Vec::new();
        }

        self.action_sent = true;
        vec![ActionRequest {
            request_id: format!("{}-shutdown", observation.observation_id),
            agent_id: observation.agent_id.clone(),
            target: "engine-1".to_string(),
            command: Command::EngineShutdown,
            expected_state_version: state_version,
            expires_at_tick: observation.delivered_tick + 3,
            correlation_id: format!("{}-corr", observation.observation_id),
        }]
    }
}
