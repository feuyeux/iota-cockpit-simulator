use std::collections::BTreeMap;

use cockpit_simulation_core::{
    action::{ActionRequest, ActionResult, ActionStatus, Command},
    error::{SimulationError, SimulationResult},
    event::ToolCallTrace,
    sensor::Observation,
    simulation::{Simulation, StepRecord},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod acp_adapter;
pub mod iota_core_adapter;
pub mod live;
pub mod multi_agent;
pub mod policy;
pub mod skill;
pub mod translation;

pub use live::{
    HumanAgentDriver, HumanBackend, HumanDecision, HumanTurnContext, HumanTurnError,
    HumanTurnEvidence, InternalStateDelta, RecordedHumanBackend, RequestedAction,
};
pub use multi_agent::{AgentActionBatch, MultiAgentCoordinator};
pub use policy::{AgentRuntimePolicy, AgentTurnError};
pub use translation::{IdentityTranslator, Translator, normalize_language, same_language};

pub const TOOL_GET_OBSERVATION: &str = "simulation.get_observation";
pub const TOOL_LIST_VISIBLE_ENTITIES: &str = "simulation.list_visible_entities";
pub const TOOL_INSPECT_SENSOR_QUALITY: &str = "simulation.inspect_sensor_quality";
pub const TOOL_REQUEST_ACTION: &str = "simulation.request_action";
pub const TOOL_GET_ACTION_RESULT: &str = "simulation.get_action_result";
pub const TOOL_GET_RUN_STATUS: &str = "simulation.get_run_status";
pub const MAX_TOOL_RESPONSE_BYTES: usize = 1_048_576;
pub const REDACTED_SECRET: &str = "[REDACTED]";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub side_effect: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequest {
    pub call_id: String,
    pub run_id: String,
    pub agent_id: String,
    pub tick: u64,
    pub tool_name: String,
    pub arguments: Value,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResponse {
    pub run_id: String,
    pub tick: u64,
    pub correlation_id: String,
    pub result: Value,
    pub error: Option<ToolError>,
}

#[derive(Debug, Default)]
pub struct LocalMcpServer {
    action_results: BTreeMap<String, ActionResult>,
    pending_actions: BTreeMap<String, ActionRequest>,
    approval_required: bool,
}

impl LocalMcpServer {
    pub fn set_approval_required(&mut self, required: bool) {
        self.approval_required = required;
    }

    pub fn approve_action(
        &mut self,
        simulation: &mut Simulation,
        request_id: &str,
    ) -> Result<ActionResult, ToolError> {
        let action = self
            .pending_actions
            .remove(request_id)
            .ok_or_else(|| ToolError {
                code: "ACTION_NOT_FOUND".to_string(),
                message: "pending action was not found".to_string(),
            })?;
        let result = simulation.submit_action(action);
        self.action_results
            .insert(result.request.request_id.clone(), result.clone());
        Ok(result)
    }

    pub fn reject_action(
        &mut self,
        simulation: &Simulation,
        request_id: &str,
        cancelled: bool,
    ) -> Result<ActionResult, ToolError> {
        let action = self
            .pending_actions
            .remove(request_id)
            .ok_or_else(|| ToolError {
                code: "ACTION_NOT_FOUND".to_string(),
                message: "pending action was not found".to_string(),
            })?;
        let result = ActionResult {
            request: action,
            status: ActionStatus::Rejected,
            error_code: Some(if cancelled {
                cockpit_simulation_core::ErrorCode::ActionCancelled
            } else {
                cockpit_simulation_core::ErrorCode::ApprovalDenied
            }),
            run_id: simulation.run_id().to_string(),
            tick: simulation.snapshot.tick,
            correlation_id: request_id.to_string(),
        };
        self.action_results
            .insert(result.request.request_id.clone(), result.clone());
        Ok(result)
    }

    pub fn cancel_pending_actions(&mut self, simulation: &Simulation) -> Vec<ActionResult> {
        let pending = std::mem::take(&mut self.pending_actions);
        pending
            .into_values()
            .map(|action| {
                let result = ActionResult {
                    request: action,
                    status: ActionStatus::Rejected,
                    error_code: Some(cockpit_simulation_core::ErrorCode::ActionCancelled),
                    run_id: simulation.run_id().to_string(),
                    tick: simulation.snapshot.tick,
                    correlation_id: "agent-turn-cancelled".to_string(),
                };
                self.action_results
                    .insert(result.request.request_id.clone(), result.clone());
                result
            })
            .collect()
    }

    pub fn tool_definitions() -> Vec<ToolDefinition> {
        vec![
            definition(
                TOOL_GET_OBSERVATION,
                "Read the agent's authorized perceived world observation.",
                false,
                json!({
                    "type": "object",
                    "properties": { "sensorId": { "type": "string" } },
                    "additionalProperties": false
                }),
            ),
            definition(
                TOOL_LIST_VISIBLE_ENTITIES,
                "List entities visible to the agent sensor profile.",
                false,
                object_schema(),
            ),
            definition(
                TOOL_INSPECT_SENSOR_QUALITY,
                "Inspect confidence and quality for the current sensor observation.",
                false,
                object_schema(),
            ),
            definition(
                TOOL_REQUEST_ACTION,
                "Request one typed action through the Action Gateway.",
                true,
                json!({
                    "type": "object",
                    "required": ["target", "command", "expectedStateVersion", "expiresAtTick"],
                    "properties": {
                        "target": { "type": "string" },
                        "command": { "type": "string", "enum": [
                            "engineShutdown",
                            "alarmActivate",
                            "climateComfortRestore",
                            "windshieldDefogActivate",
                            "fatigueInterventionActivate",
                            "childProtectionActivate",
                            "medicalResponseActivate",
                            "privacyModeActivate",
                            "chargingPlanAccept",
                            "adasTakeoverAcknowledge",
                            "cyberSafeModeActivate"
                        ] },
                        "expectedStateVersion": { "type": "integer", "minimum": 0 },
                        "expiresAtTick": { "type": "integer", "minimum": 0 }
                    },
                    "additionalProperties": false
                }),
            ),
            definition(
                TOOL_GET_ACTION_RESULT,
                "Read the result of a previously requested action.",
                false,
                json!({
                    "type": "object",
                    "required": ["requestId"],
                    "properties": { "requestId": { "type": "string" } },
                    "additionalProperties": false
                }),
            ),
            definition(
                TOOL_GET_RUN_STATUS,
                "Read run status, tick, simulation time, and state version.",
                false,
                object_schema(),
            ),
        ]
    }

    pub fn call(
        &mut self,
        simulation: &mut Simulation,
        request: ToolRequest,
    ) -> (ToolResponse, ToolCallTrace) {
        let side_effect = request.tool_name == TOOL_REQUEST_ACTION;
        let mut allowed = true;
        let result = if request.run_id != simulation.run_id() {
            allowed = false;
            Err(ToolError {
                code: "RUN_NOT_FOUND".to_string(),
                message: "tool request runId does not match the active run".to_string(),
            })
        } else if request.agent_id != simulation.scenario.agent.agent_id {
            allowed = false;
            Err(ToolError {
                code: "AGENT_IDENTITY_DENIED".to_string(),
                message: "agent identity is not authorized for this run".to_string(),
            })
        } else {
            self.dispatch(simulation, &request)
        };

        let (result, error) = match result {
            Ok(result) if response_fits(&result) => (result, None),
            Ok(_) => (
                Value::Null,
                Some(ToolError {
                    code: "PAYLOAD_TOO_LARGE".to_string(),
                    message: format!(
                        "tool response exceeds {MAX_TOOL_RESPONSE_BYTES} byte limit; use a paginated tool response"
                    ),
                }),
            ),
            Err(error) => (Value::Null, Some(error)),
        };
        let response = ToolResponse {
            run_id: simulation.run_id().to_string(),
            tick: simulation.snapshot.tick,
            correlation_id: request.correlation_id.clone(),
            result,
            error,
        };
        let trace = ToolCallTrace {
            call_id: request.call_id,
            tool_name: request.tool_name,
            run_id: simulation.run_id().to_string(),
            agent_id: request.agent_id,
            tick: simulation.snapshot.tick,
            correlation_id: request.correlation_id,
            arguments: redact_json(request.arguments),
            result: redact_json(serde_json::to_value(&response).unwrap_or(Value::Null)),
            side_effect,
            allowed,
        };
        (response, trace)
    }

    fn dispatch(
        &mut self,
        simulation: &mut Simulation,
        request: &ToolRequest,
    ) -> Result<Value, ToolError> {
        match request.tool_name.as_str() {
            TOOL_GET_OBSERVATION => {
                serde_json::to_value(simulation.observation()).map_err(serialization_error)
            }
            TOOL_LIST_VISIBLE_ENTITIES => {
                let observation = simulation.observation();
                Ok(
                    json!({ "runId": simulation.run_id(), "tick": simulation.snapshot.tick, "entities": observation.visible_entities }),
                )
            }
            TOOL_INSPECT_SENSOR_QUALITY => {
                let observation = simulation.observation();
                Ok(
                    json!({ "runId": simulation.run_id(), "tick": simulation.snapshot.tick, "quality": observation.quality }),
                )
            }
            TOOL_REQUEST_ACTION => self.request_action(simulation, request),
            TOOL_GET_ACTION_RESULT => {
                let request_id = request
                    .arguments
                    .get("requestId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid_arguments("requestId is required"))?;
                self.action_results
                    .get(request_id)
                    .map(|result| serde_json::to_value(result).unwrap_or(Value::Null))
                    .ok_or_else(|| ToolError {
                        code: "ACTION_NOT_FOUND".to_string(),
                        message: "action result was not found".to_string(),
                    })
            }
            TOOL_GET_RUN_STATUS => Ok(json!({
                "runId": simulation.run_id(),
                "status": simulation.status,
                "tick": simulation.snapshot.tick,
                "simTimeMs": simulation.snapshot.sim_time_ms,
                "stateVersion": simulation.snapshot.version
            })),
            _ => Err(ToolError {
                code: "TOOL_NOT_FOUND".to_string(),
                message: "tool is not registered".to_string(),
            }),
        }
    }

    fn request_action(
        &mut self,
        simulation: &mut Simulation,
        request: &ToolRequest,
    ) -> Result<Value, ToolError> {
        let target = request
            .arguments
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_arguments("target is required"))?;
        let command = request
            .arguments
            .get("command")
            .and_then(Value::as_str)
            .and_then(Command::from_wire_name)
            .ok_or_else(|| invalid_arguments("command is not a registered cockpit action"))?;
        let expected_state_version = request
            .arguments
            .get("expectedStateVersion")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid_arguments("expectedStateVersion is required"))?;
        let expires_at_tick = request
            .arguments
            .get("expiresAtTick")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid_arguments("expiresAtTick is required"))?;
        let action = ActionRequest {
            request_id: request.call_id.clone(),
            agent_id: request.agent_id.clone(),
            target: target.to_string(),
            command,
            expected_state_version,
            expires_at_tick,
            correlation_id: request.correlation_id.clone(),
        };
        if self.approval_required {
            let result = ActionResult {
                request: action.clone(),
                status: ActionStatus::PendingApproval,
                error_code: None,
                run_id: simulation.run_id().to_string(),
                tick: simulation.snapshot.tick,
                correlation_id: request.correlation_id.clone(),
            };
            self.pending_actions
                .insert(action.request_id.clone(), action);
            self.action_results
                .insert(result.request.request_id.clone(), result.clone());
            return serde_json::to_value(result).map_err(serialization_error);
        }
        let result = simulation.submit_action(action);
        self.action_results
            .insert(result.request.request_id.clone(), result.clone());
        serde_json::to_value(result).map_err(serialization_error)
    }
}

pub fn redact_json(mut value: Value) -> Value {
    redact_json_in_place(&mut value);
    value
}

fn redact_json_in_place(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(redact_json_in_place),
        Value::Object(values) => {
            for (key, value) in values {
                if sensitive_key(key) {
                    *value = Value::String(REDACTED_SECRET.to_string());
                } else {
                    redact_json_in_place(value);
                }
            }
        }
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey"
            | "token"
            | "authorization"
            | "password"
            | "secret"
            | "credential"
            | "credentials"
            | "prompt"
            | "reasoning"
            | "hiddenreasoning"
            | "chainofthought"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("token")
        || normalized.ends_with("secret")
        || normalized.ends_with("password")
        || normalized.ends_with("credential")
        || normalized.ends_with("credentials")
        || normalized.ends_with("prompt")
}

fn response_fits(result: &Value) -> bool {
    serde_json::to_vec(result)
        .map(|bytes| bytes.len() <= MAX_TOOL_RESPONSE_BYTES)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{MAX_TOOL_RESPONSE_BYTES, REDACTED_SECRET, redact_json, response_fits};
    use serde_json::json;

    #[test]
    fn tool_response_size_limit_rejects_oversized_values_without_truncation() {
        let value = json!({ "payload": "x".repeat(MAX_TOOL_RESPONSE_BYTES) });
        assert!(!response_fits(&value));
    }

    #[test]
    fn trace_redaction_removes_nested_secret_values() {
        let value = redact_json(json!({
            "outer": {
                "apiKey": "do-not-leak",
                "nested": [{ "auth_token": "also-do-not-leak" }]
            }
        }));
        assert_eq!(value["outer"]["apiKey"], REDACTED_SECRET);
        assert_eq!(value["outer"]["nested"][0]["auth_token"], REDACTED_SECRET);
        assert!(!value.to_string().contains("do-not-leak"));
    }

    #[test]
    fn trace_redaction_removes_credential_values() {
        let value = redact_json(json!({
            "credential": "do-not-leak",
            "awsCredentials": "also-do-not-leak"
        }));
        assert_eq!(value["credential"], REDACTED_SECRET);
        assert_eq!(value["awsCredentials"], REDACTED_SECRET);
        assert!(!value.to_string().contains("do-not-leak"));
    }
}

#[derive(Debug, Default)]
pub struct RuleAgent {
    sequence: u64,
    handled_alerts: std::collections::BTreeSet<String>,
}

impl RuleAgent {
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        server: &mut LocalMcpServer,
    ) -> SimulationResult<StepRecord> {
        self.step_with_state_diffs(simulation, server, Vec::new())
    }

    pub fn step_with_state_diffs(
        &mut self,
        simulation: &mut Simulation,
        server: &mut LocalMcpServer,
        state_diffs: Vec<cockpit_simulation_core::StateDiff>,
    ) -> SimulationResult<StepRecord> {
        let observation_request = self.request(simulation, TOOL_GET_OBSERVATION, json!({}));
        let (observation_response, observation_trace) =
            server.call(simulation, observation_request);
        let observation: Observation = serde_json::from_value(observation_response.result)
            .map_err(|err| SimulationError::Serialization(err.to_string()))?;
        let mut traces = vec![observation_trace];

        let action_for_alert = |alert: &str| match alert {
            "SmokeDetected" => Some(("engine-1", "engineShutdown")),
            "ThermalComfortRisk" => Some(("hvac-1", "climateComfortRestore")),
            "WindshieldVisibilityRisk" => Some(("defogger-1", "windshieldDefogActivate")),
            "DriverFatigueRisk" => Some(("dms-1", "fatigueInterventionActivate")),
            "ChildPresenceHeatRisk" => Some(("occupant-radar-1", "childProtectionActivate")),
            "MedicalEmergencyRisk" => Some(("emergency-call-1", "medicalResponseActivate")),
            "MultiUserPrivacyConflict" => Some(("voice-array-1", "privacyModeActivate")),
            "EvRangeRisk" => Some(("navigation-1", "chargingPlanAccept")),
            "AdasTakeoverRequired" => Some(("adas-controller-1", "adasTakeoverAcknowledge")),
            "CyberControlAnomaly" => Some(("security-monitor-1", "cyberSafeModeActivate")),
            _ => None,
        };
        for alert in &observation.alerts {
            let Some((target, command)) = action_for_alert(alert) else {
                continue;
            };
            if !self.handled_alerts.insert(alert.clone()) {
                continue;
            }
            let action_request = self.request(
                simulation,
                TOOL_REQUEST_ACTION,
                json!({
                    "target": target,
                    "command": command,
                    "expectedStateVersion": simulation.snapshot.version,
                    "expiresAtTick": simulation.snapshot.tick + 3
                }),
            );
            let (_, action_trace) = server.call(simulation, action_request);
            traces.push(action_trace);
        }

        let mut step = simulation.step_with_state_diffs(state_diffs)?;
        step.tool_calls = traces;
        Ok(step)
    }

    fn request(
        &mut self,
        simulation: &Simulation,
        tool_name: &str,
        arguments: Value,
    ) -> ToolRequest {
        self.sequence += 1;
        ToolRequest {
            call_id: format!("{}-tool-{}", simulation.run_id(), self.sequence),
            run_id: simulation.run_id().to_string(),
            agent_id: simulation.scenario.agent.agent_id.clone(),
            tick: simulation.snapshot.tick,
            tool_name: tool_name.to_string(),
            arguments,
            correlation_id: format!("{}-corr-{}", simulation.run_id(), self.sequence),
        }
    }
}

fn definition(
    name: &str,
    description: &str,
    side_effect: bool,
    input_schema: Value,
) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
        side_effect,
    }
}

fn object_schema() -> Value {
    json!({ "type": "object", "additionalProperties": false })
}

fn invalid_arguments(message: &str) -> ToolError {
    ToolError {
        code: "INVALID_ARGUMENTS".to_string(),
        message: message.to_string(),
    }
}

fn serialization_error(error: serde_json::Error) -> ToolError {
    ToolError {
        code: "SERIALIZATION_ERROR".to_string(),
        message: error.to_string(),
    }
}
