use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPayload {
    pub message: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub run_id: String,
    pub tick: u64,
    pub source: String,
    pub priority: i32,
    pub sequence: u64,
    pub correlation_id: String,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallTrace {
    pub call_id: String,
    pub tool_name: String,
    pub run_id: String,
    pub agent_id: String,
    pub tick: u64,
    pub correlation_id: String,
    pub arguments: Value,
    pub result: Value,
    pub side_effect: bool,
    pub allowed: bool,
}
