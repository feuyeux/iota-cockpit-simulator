//! Wire/decision types, the [`HumanBackend`] trait, and
//! [`RecordedHumanBackend`] for the per-human decision driver.
//!
//! See the [`super`] module docs for the overall tick/tool-loop contract.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use cockpit_world::{NeedsState, PerceivedEvent, Persona, simulation::Simulation};

use crate::{LocalMcpServer, ToolResponse, native_mcp::NativeMcpCall};

use super::IMPLICIT_NARRATIVE;

/// Bounded numeric adjustment to a human's dynamic state, applied after range
/// validation. Fields are optional deltas; an absent field leaves that value
/// unchanged this tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalStateDelta {
    #[serde(default)]
    pub stress: Option<f64>,
    #[serde(default)]
    pub attention: Option<f64>,
}

/// One requested action in a backend-authored decision, shaped like the
/// existing MCP `simulation.request_action` tool arguments so it can be
/// converted into an [`cockpit_world::ActionRequest`] and validated
/// by the unchanged Action Gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestedAction {
    pub target: String,
    pub command: String,
}

/// One model-requested simulation tool call. Identity, run, tick, and call IDs
/// are injected by the runtime and therefore cannot be spoofed by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanToolCall {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

/// One completed tool exchange returned to the backend on its next round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanToolExchange {
    pub call_id: String,
    pub call: HumanToolCall,
    pub response: ToolResponse,
}

#[derive(Debug)]
pub(super) enum HumanTurnOutput {
    ToolCall(HumanToolCall),
    Final(HumanDecision),
}

/// The structured decision a backend returns for a human's turn. `utterance`,
/// `actions`, and `internalStateDelta` are optional per-tick outputs.
/// `narrative` is normalized when omitted by a backend.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanDecision {
    #[serde(default)]
    pub utterance: Option<String>,
    #[serde(default)]
    pub actions: Vec<RequestedAction>,
    #[serde(default)]
    pub internal_state_delta: InternalStateDelta,
    pub narrative: String,
}

/// Fatal error from a mandatory per-human backend turn. The caller must
/// propagate this to fail the run; there is no fallback path.
#[derive(Debug, thiserror::Error)]
pub enum HumanTurnError {
    #[error("backend turn failed for human {human_id}: {reason}")]
    Backend { human_id: String, reason: String },
    #[error("backend returned invalid decision output for human {human_id}: {reason}")]
    InvalidOutput { human_id: String, reason: String },
}

/// Per-tick record of one human's backend-driven decision, kept alongside the
/// deterministic [`cockpit_world::simulation::StepRecord`] as
/// recording/replay evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanTurnEvidence {
    pub human_id: String,
    pub decision: HumanDecision,
    #[serde(default)]
    pub tool_calls: Vec<HumanToolCall>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

/// The per-human, per-tick context handed to the backend. Bundles the human's
/// stable persona, current needs/goal, delivered perception (physical + social),
/// long-term memory, and completed tool exchanges. No eager world observation
/// is present; dynamic world data enters only through human-scoped tools.
#[derive(Debug, Clone)]
pub struct HumanTurnContext {
    pub human_id: String,
    pub persona: Persona,
    pub needs: NeedsState,
    pub goal: String,
    /// Perception delivered to this human as of the current tick (physical
    /// events + others' utterances whose delay has elapsed).
    pub delivered_perception: Vec<PerceivedEvent>,
    pub long_term_memory: Vec<String>,
    /// Capability names this human is authorized to propose, from its world
    /// grant. The prompt lists only the matching action commands so the backend
    /// is never offered a command it would be rejected for proposing.
    pub action_capabilities: Vec<String>,
    /// Tool results accumulated during this person's current tick. The first
    /// backend round receives an empty history and must query what it needs.
    pub tool_history: Vec<HumanToolExchange>,
    pub round: usize,
    /// Language tag ("en"/"zh") the backend should respond in, from the
    /// scenario's `language`. Other languages are produced on demand by
    /// translation, not by re-running the backend.
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendConversationUpdate {
    pub backend: String,
    pub backend_session_id: Option<String>,
    pub response_kind: String,
    pub tool_name: Option<String>,
}

/// A backend that produces one machine-readable output for the current round.
/// Implementors own backend selection (real ACP, synthetic, or recorded); the
/// driver calls repeatedly until `final` or the tool budget is exhausted.
pub trait HumanBackend {
    /// Prepare an authenticated native MCP transaction for this human. Text
    /// protocol, synthetic, and replay backends use the default no-op.
    fn prepare_native_tools(
        &mut self,
        _simulation: &Simulation,
        _server: &LocalMcpServer,
        _context: &HumanTurnContext,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Drain native MCP calls made inside the most recent ACP turn. The driver
    /// replays them through its cloned LocalMcpServer before committing.
    fn take_native_tool_calls(&mut self) -> Result<Vec<NativeMcpCall>, String> {
        Ok(Vec::new())
    }

    fn take_conversation_update(&mut self) -> Option<BackendConversationUpdate> {
        None
    }

    /// Run one mandatory backend round. An `Err` is fatal for the uncommitted
    /// tick under the mandatory-backend contract.
    fn run_turn(
        &mut self,
        context: &HumanTurnContext,
    ) -> impl std::future::Future<Output = Result<String, String>>;
}

/// A backend that replays previously recorded tool calls and final dispositions
/// instead of calling a real model. Feeding the transcript through the same
/// driver preserves the Action Gateway, social perception, and state-delta
/// boundaries without another model call.
pub struct RecordedHumanBackend {
    outputs: std::collections::VecDeque<String>,
}

impl RecordedHumanBackend {
    /// Build a deterministic backend transcript in driver consumption order:
    /// tick, human, tool calls, then final. Tool-driven recordings clear the
    /// synthesized final `actions` field because those actions are replayed by
    /// their original `simulation.request_action` calls.
    pub fn from_tick_evidence(ticks: &[Vec<HumanTurnEvidence>]) -> Self {
        let outputs = ticks
            .iter()
            .flat_map(|tick| tick.iter())
            .flat_map(|evidence| {
                let mut outputs = evidence
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "type": "toolCall",
                            "tool": call.tool.clone(),
                            "arguments": call.arguments.clone()
                        })
                        .to_string()
                    })
                    .collect::<Vec<_>>();
                let mut decision = evidence.decision.clone();
                if !evidence.tool_calls.is_empty() {
                    decision.actions.clear();
                }
                let mut final_value = serde_json::to_value(decision)
                    .unwrap_or_else(|_| serde_json::json!({ "narrative": IMPLICIT_NARRATIVE }));
                if let Some(object) = final_value.as_object_mut() {
                    object.insert("type".to_string(), Value::String("final".to_string()));
                }
                outputs.push(final_value.to_string());
                outputs
            })
            .collect();
        Self { outputs }
    }
}

impl HumanBackend for RecordedHumanBackend {
    async fn run_turn(&mut self, _context: &HumanTurnContext) -> Result<String, String> {
        self.outputs
            .pop_front()
            .ok_or_else(|| "recorded backend exhausted its outputs during replay".to_string())
    }
}
