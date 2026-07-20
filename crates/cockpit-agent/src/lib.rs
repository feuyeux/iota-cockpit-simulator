use std::collections::BTreeMap;

use cockpit_world::{
    action::{ActionRequest, ActionResult, ActionStatus},
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
pub mod native_mcp;
pub mod open_world;
pub mod policy;
pub mod skill;
pub mod translation;

pub use live::{
    BackendConversationUpdate, HumanAgentDriver, HumanBackend, HumanDecision, HumanToolCall,
    HumanToolExchange, HumanTurnContext, HumanTurnError, HumanTurnEvidence, InternalStateDelta,
    RecordedHumanBackend, RequestedAction,
};
pub use multi_agent::{AgentActionBatch, MultiAgentCoordinator};
pub use open_world::{
    AcpConversationTurn, AgentBudget, AgentLifecycle, AgentSessionState, EpisodicMemory, GoalState,
    GoalStatus, OpenWorldCheckpoint, OpenWorldRuntime, PlanStep, PlanStepStatus, RelationshipState,
    ResourceLifecycle, SkillState, ToolState,
};
pub use policy::{AgentRuntimePolicy, AgentTurnError};
pub use translation::{IdentityTranslator, Translator, normalize_language, same_language};

pub const TOOL_GET_OBSERVATION: &str = "simulation.get_observation";
pub const TOOL_GET_TURN_CONTEXT: &str = "simulation.get_turn_context";
pub const TOOL_LIST_VISIBLE_ENTITIES: &str = "simulation.list_visible_entities";
pub const TOOL_INSPECT_SENSOR_QUALITY: &str = "simulation.inspect_sensor_quality";
pub const TOOL_REQUEST_ACTION: &str = "simulation.request_action";
pub const TOOL_SUBMIT_DECISION: &str = "simulation.submit_decision";
pub const TOOL_GET_ACTION_RESULT: &str = "simulation.get_action_result";
pub const TOOL_GET_RUN_STATUS: &str = "simulation.get_run_status";
pub const TOOL_ADD_GOAL: &str = "simulation.add_goal";
pub const TOOL_WAIT_UNTIL: &str = "simulation.wait_until";
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
    /// Runtime-authenticated human scope. The model never supplies this;
    /// the live driver injects it for perception and capability isolation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_id: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OpenWorldControlRequest {
    AddGoal {
        human_id: String,
        description: String,
        priority: i32,
    },
    WaitUntil {
        human_id: String,
        wake_tick: u64,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalMcpServer {
    action_results: BTreeMap<String, ActionResult>,
    action_owners: BTreeMap<String, Option<String>>,
    pending_actions: BTreeMap<String, ActionRequest>,
    #[serde(default)]
    control_requests: Vec<OpenWorldControlRequest>,
    approval_required: bool,
}

impl LocalMcpServer {
    pub fn set_approval_required(&mut self, required: bool) {
        self.approval_required = required;
    }

    pub fn take_control_requests(&mut self) -> Vec<OpenWorldControlRequest> {
        std::mem::take(&mut self.control_requests)
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
                cockpit_world::ErrorCode::ActionCancelled
            } else {
                cockpit_world::ErrorCode::ApprovalDenied
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
                    error_code: Some(cockpit_world::ErrorCode::ActionCancelled),
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
                TOOL_GET_TURN_CONTEXT,
                "Read the authenticated human's complete authorized turn context: observation, sensor quality, and run status. Prefer this at the start of a turn over separate read-only queries.",
                false,
                object_schema(),
            ),
            definition(
                TOOL_GET_OBSERVATION,
                "Read the agent's authorized perceived world observation.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "sensorId": { "type": "string" },
                        "cursor": { "type": "integer", "minimum": 0 },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
                    },
                    "additionalProperties": false
                }),
            ),
            definition(
                TOOL_LIST_VISIBLE_ENTITIES,
                "List entities visible to the agent sensor profile using cursor pagination.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "cursor": { "type": "integer", "minimum": 0 },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
                    },
                    "additionalProperties": false
                }),
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
                        "command": { "type": "string", "enum":
                            cockpit_world::capability::CapabilityCatalog::load_default()
                                .definitions()
                                .map(|capability| Value::String(capability.wire_name.clone()))
                                .collect::<Vec<_>>()
                        },
                        "expectedStateVersion": { "type": "integer", "minimum": 0 },
                        "expiresAtTick": { "type": "integer", "minimum": 0 }
                    },
                    "additionalProperties": false
                }),
            ),
            definition(
                TOOL_SUBMIT_DECISION,
                "Submit the final structured human disposition. Call exactly once as the final native tool of the turn.",
                false,
                json!({
                    "type": "object",
                    "required": ["narrative"],
                    "properties": {
                        "utterance": {
                            "anyOf": [
                                { "type": "string", "maxLength": 1024 },
                                { "type": "null" }
                            ]
                        },
                        "internalStateDelta": {
                            "type": "object",
                            "properties": {
                                "stress": {
                                    "anyOf": [
                                        { "type": "number", "minimum": -0.25, "maximum": 0.25 },
                                        { "type": "null" }
                                    ]
                                },
                                "attention": {
                                    "anyOf": [
                                        { "type": "number", "minimum": -0.25, "maximum": 0.25 },
                                        { "type": "null" }
                                    ]
                                }
                            },
                            "additionalProperties": false
                        },
                        "narrative": { "type": "string", "minLength": 1, "maxLength": 1024 }
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
            definition(
                TOOL_ADD_GOAL,
                "Add a bounded goal to the authenticated human's own plan state.",
                true,
                json!({
                    "type": "object",
                    "required": ["description"],
                    "properties": {
                        "description": { "type": "string", "minLength": 1, "maxLength": 512 },
                        "priority": { "type": "integer", "minimum": -100, "maximum": 100 }
                    },
                    "additionalProperties": false
                }),
            ),
            definition(
                TOOL_WAIT_UNTIL,
                "Put the authenticated human's runtime session to sleep until a future tick.",
                true,
                json!({
                    "type": "object",
                    "required": ["wakeAtTick"],
                    "properties": {
                        "wakeAtTick": { "type": "integer", "minimum": 0 }
                    },
                    "additionalProperties": false
                }),
            ),
        ]
    }

    pub fn call(
        &mut self,
        simulation: &mut Simulation,
        request: ToolRequest,
    ) -> (ToolResponse, ToolCallTrace) {
        let side_effect = matches!(
            request.tool_name.as_str(),
            TOOL_REQUEST_ACTION | TOOL_ADD_GOAL | TOOL_WAIT_UNTIL
        );
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
        } else if let Err(error) = Self::validate_human_scope(simulation, &request) {
            allowed = false;
            Err(error)
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
            TOOL_GET_TURN_CONTEXT => {
                let observation = Self::observation_for_request(simulation, request);
                let quality = observation.quality.clone();
                let observation = serde_json::to_value(observation).map_err(serialization_error)?;
                Ok(json!({
                    "runId": simulation.run_id(),
                    "tick": simulation.snapshot.tick,
                    "observation": observation,
                    "sensorQuality": quality,
                    "status": simulation.status,
                    "simTimeMs": simulation.snapshot.sim_time_ms,
                    "stateVersion": simulation.snapshot.version
                }))
            }
            TOOL_GET_OBSERVATION => {
                let mut observation =
                    serde_json::to_value(Self::observation_for_request(simulation, request))
                        .map_err(serialization_error)?;
                let cursor = request
                    .arguments
                    .get("cursor")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let limit = request
                    .arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .clamp(1, 200) as usize;
                let entities = observation
                    .get_mut("visibleEntities")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| ToolError {
                        code: "SERIALIZATION_ERROR".to_string(),
                        message: "observation visibleEntities is not an array".to_string(),
                    })?;
                let total = entities.len();
                let page = entities
                    .iter()
                    .skip(cursor)
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>();
                let next_cursor = (cursor + page.len() < total).then_some(cursor + page.len());
                *entities = page;
                if let Some(object) = observation.as_object_mut() {
                    object.insert(
                        "page".to_string(),
                        json!({
                            "cursor": cursor,
                            "limit": limit,
                            "total": total,
                            "nextCursor": next_cursor
                        }),
                    );
                }
                Ok(observation)
            }
            TOOL_LIST_VISIBLE_ENTITIES => {
                let observation = Self::observation_for_request(simulation, request);
                let cursor = request
                    .arguments
                    .get("cursor")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let limit = request
                    .arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 200) as usize;
                let total = observation.visible_entities.len();
                let entities = observation
                    .visible_entities
                    .into_iter()
                    .skip(cursor)
                    .take(limit)
                    .collect::<Vec<_>>();
                let next_cursor =
                    (cursor + entities.len() < total).then_some(cursor + entities.len());
                Ok(json!({
                    "runId": simulation.run_id(),
                    "tick": simulation.snapshot.tick,
                    "entities": entities,
                    "page": {
                        "cursor": cursor,
                        "limit": limit,
                        "total": total,
                        "nextCursor": next_cursor
                    }
                }))
            }
            TOOL_INSPECT_SENSOR_QUALITY => {
                let observation = Self::observation_for_request(simulation, request);
                Ok(
                    json!({ "runId": simulation.run_id(), "tick": simulation.snapshot.tick, "quality": observation.quality }),
                )
            }
            TOOL_REQUEST_ACTION => self.request_action(simulation, request),
            TOOL_SUBMIT_DECISION => {
                crate::live::parse_submitted_decision(&request.arguments)
                    .map_err(|message| invalid_arguments(&message))?;
                Ok(json!({
                    "accepted": true,
                    "kind": "humanDecision",
                    "humanId": request.human_id.as_deref()
                }))
            }
            TOOL_GET_ACTION_RESULT => {
                let request_id = request
                    .arguments
                    .get("requestId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid_arguments("requestId is required"))?;
                let result = self
                    .action_results
                    .get(request_id)
                    .ok_or_else(|| ToolError {
                        code: "ACTION_NOT_FOUND".to_string(),
                        message: "action result was not found".to_string(),
                    })?;
                if request.human_id.is_some()
                    && self.action_owners.get(request_id) != Some(&request.human_id)
                {
                    return Err(ToolError {
                        code: "ACTION_RESULT_IDENTITY_DENIED".to_string(),
                        message: "action result belongs to a different human".to_string(),
                    });
                }
                serde_json::to_value(result).map_err(serialization_error)
            }
            TOOL_ADD_GOAL => {
                let human_id = request.human_id.clone().ok_or_else(|| ToolError {
                    code: "HUMAN_SCOPE_REQUIRED".to_string(),
                    message: "open-world control tools require an authenticated human".to_string(),
                })?;
                let description = request
                    .arguments
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && value.len() <= 512)
                    .ok_or_else(|| invalid_arguments("description must contain 1..=512 bytes"))?;
                let priority = request
                    .arguments
                    .get("priority")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                if !(-100..=100).contains(&priority) {
                    return Err(invalid_arguments("priority must be in -100..=100"));
                }
                self.control_requests
                    .push(OpenWorldControlRequest::AddGoal {
                        human_id,
                        description: description.to_string(),
                        priority: priority as i32,
                    });
                Ok(json!({
                    "accepted": true,
                    "controlId": request.call_id,
                    "kind": "addGoal"
                }))
            }
            TOOL_WAIT_UNTIL => {
                let human_id = request.human_id.clone().ok_or_else(|| ToolError {
                    code: "HUMAN_SCOPE_REQUIRED".to_string(),
                    message: "open-world control tools require an authenticated human".to_string(),
                })?;
                let wake_tick = request
                    .arguments
                    .get("wakeAtTick")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid_arguments("wakeAtTick is required"))?;
                if wake_tick <= simulation.snapshot.tick
                    || wake_tick > simulation.snapshot.tick.saturating_add(1_000_000)
                {
                    return Err(invalid_arguments(
                        "wakeAtTick must be a future tick within 1,000,000 ticks",
                    ));
                }
                self.control_requests
                    .push(OpenWorldControlRequest::WaitUntil {
                        human_id,
                        wake_tick,
                    });
                Ok(json!({
                    "accepted": true,
                    "controlId": request.call_id,
                    "kind": "waitUntil",
                    "wakeAtTick": wake_tick
                }))
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

    fn observation_for_request(simulation: &Simulation, request: &ToolRequest) -> Observation {
        request.human_id.as_deref().map_or_else(
            || simulation.observation(),
            |human_id| Observation::for_human(simulation.run_id(), human_id, &simulation.snapshot),
        )
    }

    fn validate_human_scope(
        simulation: &Simulation,
        request: &ToolRequest,
    ) -> Result<(), ToolError> {
        let Some(human_id) = request.human_id.as_deref() else {
            if matches!(request.tool_name.as_str(), TOOL_ADD_GOAL | TOOL_WAIT_UNTIL) {
                return Err(ToolError {
                    code: "HUMAN_SCOPE_REQUIRED".to_string(),
                    message: "open-world control tools require an authenticated human".to_string(),
                });
            }
            return Ok(());
        };
        let human = simulation
            .snapshot
            .human(human_id)
            .ok_or_else(|| ToolError {
                code: "HUMAN_IDENTITY_DENIED".to_string(),
                message: "human identity is not present in the active world".to_string(),
            })?;
        if request.tool_name != TOOL_REQUEST_ACTION {
            return Ok(());
        }
        let Some(capability) = request
            .arguments
            .get("command")
            .and_then(Value::as_str)
            .and_then(|wire_name| simulation.capabilities().get_by_wire_name(wire_name))
        else {
            return Ok(());
        };
        if human
            .action_capabilities
            .iter()
            .any(|owned| owned == &capability.id)
        {
            Ok(())
        } else {
            Err(ToolError {
                code: "HUMAN_CAPABILITY_DENIED".to_string(),
                message: "human is not authorized to request this action".to_string(),
            })
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
        let capability_id = request
            .arguments
            .get("command")
            .and_then(Value::as_str)
            .and_then(|wire_name| simulation.capabilities().get_by_wire_name(wire_name))
            .ok_or_else(|| invalid_arguments("command is not a registered cockpit action"))?
            .id
            .clone();
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
            capability_id,
            expected_state_version,
            expires_at_tick,
            correlation_id: request.correlation_id.clone(),
        };
        self.action_owners
            .insert(action.request_id.clone(), request.human_id.clone());
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
        state_diffs: Vec<cockpit_world::StateDiff>,
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
            human_id: None,
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
