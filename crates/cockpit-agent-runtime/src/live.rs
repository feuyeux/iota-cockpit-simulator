//! Per-human, backend-mandatory decision driver.
//!
//! Each tick, every human in the scenario gets exactly one backend turn: the
//! backend (hermes, etc.) is given that human's persona, needs, and delivered
//! perception (physical + social), and must return a structured decision. The
//! decision may include:
//! - `actions`: typed [`ActionRequest`]s submitted through the existing Action
//!   Gateway (capability/version/precondition checks are unchanged).
//! - `internalStateDelta`: bounded numeric adjustments to stress/attention,
//!   applied after range validation.
//! - `utterance`: text enqueued into every other human's social perception
//!   queue for delivery on a later tick (never the same tick).
//! - `narrative`: optional free-form reasoning evidence, redacted before it
//!   reaches durable state. A missing narrative is normalized to a fixed
//!   placeholder because it never influences simulation behavior.
//!
//! There is no fallback: if the backend fails, times out, or returns output
//! malformed structured output, [`HumanAgentDriver::step_with_backend`] returns
//! `Err` and the caller must fail the run. Missing narrative prose alone is
//! normalized because no decision effect depends on it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use cockpit_simulation_core::{
    ActionRequest, Command, NeedsState, PerceivedEvent, Persona,
    error::SimulationError,
    perception::{delivered_and_pending, enqueue_social_event, perception_delay_ticks},
    sensor::Observation,
    simulation::{HumanStateDelta, Simulation, StepRecord},
};

const REDACTED_DECISION_TEXT: &str = "[REDACTED]";
const IMPLICIT_NARRATIVE: &str = "implicit backend decision";
const MAX_ACTIONS_PER_DECISION: usize = 4;
const MAX_DECISION_TEXT_BYTES: usize = 1_024;
const MAX_STATE_DELTA_MAGNITUDE: f64 = 0.25;

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
/// converted into an [`ActionRequest`] and validated by the unchanged Action
/// Gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestedAction {
    pub target: String,
    pub command: String,
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
/// deterministic [`StepRecord`] as recording/replay evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanTurnEvidence {
    pub human_id: String,
    pub decision: HumanDecision,
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

/// The per-human, per-tick context handed to the backend. Bundles the human's
/// stable persona, current needs/goal, the perception delivered to them this
/// tick (physical + social), and their long-term memory, alongside the
/// authorized [`Observation`]. This is what makes a decision persona-aware:
/// the backend prompt is built entirely from resource-driven persona data plus
/// this dynamic state, never from Ground Truth.
#[derive(Debug, Clone)]
pub struct HumanTurnContext {
    pub human_id: String,
    pub observation: Observation,
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
    /// Language tag ("en"/"zh") the backend should respond in, from the
    /// scenario's `language`. Other languages are produced on demand by
    /// translation, not by re-running the backend.
    pub language: String,
}

/// A backend that produces one human's decision text for a tick. Implementors
/// own backend selection (real ACP backend, synthetic offline stand-in, or a
/// per-human mix); the driver only needs the raw response text to parse into a
/// [`HumanDecision`]. Taking `&mut self` lets an implementor hold a live
/// session; the driver calls this sequentially, one human at a time.
pub trait HumanBackend {
    /// Run one mandatory backend turn for the human described by `context`,
    /// resolving to the backend's raw response text. An `Err` is fatal for the
    /// run under the mandatory-backend contract.
    fn run_turn(
        &mut self,
        context: &HumanTurnContext,
    ) -> impl std::future::Future<Output = Result<String, String>>;
}

/// A backend that replays previously recorded [`HumanDecision`]s in order,
/// instead of calling a real model. This is what makes a *live* run replayable
/// and deterministic: the original run's per-human decisions are recorded, and
/// replay feeds them back through the exact same [`HumanAgentDriver`] logic
/// (action gateway, social perception, state deltas) without any backend call.
/// Decisions are consumed in recording order, which matches the driver's
/// per-tick, per-human iteration order for the same scenario.
pub struct RecordedHumanBackend {
    decisions: std::collections::VecDeque<HumanDecision>,
}

impl RecordedHumanBackend {
    /// Build from recorded per-tick evidence (as stored in a recording),
    /// flattening to the driver's consumption order: tick ascending, then the
    /// human order within each tick.
    pub fn from_tick_evidence(ticks: &[Vec<HumanTurnEvidence>]) -> Self {
        let decisions = ticks
            .iter()
            .flat_map(|tick| tick.iter().map(|evidence| evidence.decision.clone()))
            .collect();
        Self { decisions }
    }
}

impl HumanBackend for RecordedHumanBackend {
    async fn run_turn(&mut self, _context: &HumanTurnContext) -> Result<String, String> {
        let decision = self
            .decisions
            .pop_front()
            .ok_or_else(|| "recorded backend exhausted its decisions during replay".to_string())?;
        serde_json::to_string(&decision)
            .map_err(|error| format!("failed to re-serialize recorded decision: {error}"))
    }
}

/// Drives one deterministic tick with a mandatory backend turn for every
/// human. Every human's decision must come from a real backend call; any
/// failure aborts the tick (and the caller should fail the run) before any
/// state is committed.
#[derive(Default)]
pub struct HumanAgentDriver {
    /// Raw utterances exist only for the lifetime of a live driver. The world
    /// snapshot keeps redacted markers so recordings and hashes never contain
    /// backend prose, while later live turns still hear what was actually said.
    transient_utterances: BTreeMap<String, Vec<PerceivedEvent>>,
}

impl HumanAgentDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run one mandatory backend turn per human via `backend`, apply their
    /// decisions, then commit the deterministic tick. The `backend` is invoked
    /// once per human, in scenario-defined human order, with that human's
    /// [`Observation`]; its response text is parsed as a [`HumanDecision`].
    ///
    /// Returns `Err` without committing the tick if any human's backend turn
    /// fails or returns invalid output. Backends that support cancellation
    /// should surface it as an `Err` from `run_turn`; this driver treats any
    /// `Err` identically as fatal, since the mandatory-backend contract has no
    /// fallback distinction between "failed" and "cancelled" at this layer.
    pub async fn step_with_backend<B: HumanBackend>(
        &mut self,
        simulation: &mut Simulation,
        backend: &mut B,
    ) -> Result<(StepRecord, Vec<HumanTurnEvidence>), HumanTurnError> {
        self.prune_transient_utterances(simulation);
        let human_ids: Vec<String> = simulation
            .snapshot
            .humans
            .iter()
            .map(|human| human.id.clone())
            .collect();

        let mut evidence = Vec::with_capacity(human_ids.len());
        let mut pending_actions: Vec<ActionRequest> = Vec::new();
        let mut pending_utterances: Vec<(String, String, String)> = Vec::new(); // (speaker_id, location, text)
        let mut deltas: Vec<(String, InternalStateDelta)> = Vec::new();

        for human_id in &human_ids {
            let observation =
                Observation::for_human(simulation.run_id(), human_id, &simulation.snapshot);
            let current_tick = simulation.snapshot.tick;
            let context = {
                let human = simulation.snapshot.human(human_id).ok_or_else(|| {
                    HumanTurnError::InvalidOutput {
                        human_id: human_id.clone(),
                        reason: "human vanished from the snapshot before its turn".to_string(),
                    }
                })?;
                let (mut delivered, _pending) = delivered_and_pending(human, current_tick);
                self.restore_transient_utterance_text(human_id, &mut delivered);
                HumanTurnContext {
                    human_id: human_id.clone(),
                    observation,
                    persona: human.persona.clone(),
                    needs: human.needs,
                    goal: human.goal.clone(),
                    delivered_perception: delivered,
                    long_term_memory: human.long_term_memory.clone(),
                    action_capabilities: human.action_capabilities.clone(),
                    language: simulation.scenario.language.clone(),
                }
            };
            let turn_started = std::time::Instant::now();
            let text =
                backend
                    .run_turn(&context)
                    .await
                    .map_err(|reason| HumanTurnError::Backend {
                        human_id: human_id.clone(),
                        reason,
                    })?;

            let mut decision =
                parse_decision(&text).map_err(|reason| HumanTurnError::InvalidOutput {
                    human_id: human_id.clone(),
                    reason,
                })?;
            sanitize_decision(&mut decision);
            let latency_ms = Some(turn_started.elapsed().as_millis() as u64);
            let transient_utterance = decision.utterance.clone();
            redact_decision_prose(&mut decision);

            let location = simulation
                .snapshot
                .human(human_id)
                .map(|human| human.location.clone())
                .unwrap_or_default();

            for (action_index, action) in decision.actions.iter().enumerate() {
                // An unknown or unauthorized command is a proposal the human was
                // never entitled to make; drop it and keep going rather than
                // failing the whole run. The gateway would reject such an action
                // anyway, and the full decision (including the dropped action)
                // is still preserved in this turn's evidence for audit.
                let Some(command) = Command::from_wire_name(&action.command) else {
                    continue;
                };
                let Some(human) = simulation.snapshot.human(human_id) else {
                    // The human was present when this turn began, but an
                    // `await` boundary sits between that snapshot read and
                    // here. Treat a since-removed human the same way as an
                    // unauthorized command: drop the action and keep going
                    // rather than panicking mid-tick.
                    continue;
                };
                if !human
                    .action_capabilities
                    .iter()
                    .any(|capability| capability == command.capability_name())
                {
                    continue;
                }
                pending_actions.push(ActionRequest {
                    request_id: format!(
                        "{}-human-{}-tick-{}-action-{}",
                        simulation.run_id(),
                        human_id,
                        simulation.snapshot.tick,
                        action_index
                    ),
                    // Human turns are dispatched by the configured cockpit
                    // agent grant. Keep the human identity in the request ID
                    // and evidence, while using the grant owner for gateway
                    // authorization.
                    agent_id: simulation.scenario.agent.agent_id.clone(),
                    target: action.target.clone(),
                    command,
                    expected_state_version: simulation.snapshot.version,
                    expires_at_tick: simulation.snapshot.tick + 3,
                    correlation_id: format!(
                        "{}-human-{}-tick-{}-action-{}-corr",
                        simulation.run_id(),
                        human_id,
                        simulation.snapshot.tick,
                        action_index
                    ),
                });
            }

            if let Some(utterance) = transient_utterance {
                pending_utterances.push((human_id.clone(), location, utterance));
            }
            deltas.push((human_id.clone(), decision.internal_state_delta));

            evidence.push(HumanTurnEvidence {
                human_id: human_id.clone(),
                decision,
                latency_ms,
            });
        }

        for action in pending_actions {
            simulation.submit_action(action);
        }

        for (human_id, delta) in deltas {
            simulation
                .submit_human_state_delta(HumanStateDelta {
                    human_id: human_id.clone(),
                    stress_delta: delta.stress,
                    attention_delta: delta.attention,
                })
                .map_err(|error| HumanTurnError::InvalidOutput {
                    human_id,
                    reason: error.to_string(),
                })?;
        }

        let tick = simulation.snapshot.tick;
        for (speaker_id, location, text) in pending_utterances {
            self.remember_transient_utterance(simulation, tick, &speaker_id, &location, &text);
            enqueue_social_event(
                &mut simulation.snapshot,
                tick,
                &speaker_id,
                &location,
                REDACTED_DECISION_TEXT,
            );
        }

        let step =
            simulation
                .step_without_agent()
                .map_err(|error| HumanTurnError::InvalidOutput {
                    human_id: "simulation".to_string(),
                    reason: error.to_string(),
                })?;
        Ok((step, evidence))
    }

    fn remember_transient_utterance(
        &mut self,
        simulation: &Simulation,
        origin_tick: u64,
        speaker_id: &str,
        speaker_location: &str,
        text: &str,
    ) {
        if text == REDACTED_DECISION_TEXT {
            return;
        }
        for human in simulation
            .snapshot
            .humans
            .iter()
            .filter(|human| human.id != speaker_id)
        {
            let delay = perception_delay_ticks(human, speaker_location).max(1);
            self.transient_utterances
                .entry(human.id.clone())
                .or_default()
                .push(PerceivedEvent {
                    origin_tick,
                    available_at_tick: origin_tick + delay,
                    source: speaker_id.to_string(),
                    kind: "utterance".to_string(),
                    summary: text.to_string(),
                });
        }
    }

    fn restore_transient_utterance_text(&self, human_id: &str, delivered: &mut [PerceivedEvent]) {
        let Some(transient) = self.transient_utterances.get(human_id) else {
            return;
        };
        for event in delivered
            .iter_mut()
            .filter(|event| event.kind == "utterance" && event.summary == REDACTED_DECISION_TEXT)
        {
            if let Some(raw) = transient
                .iter()
                .find(|raw| same_utterance_event(raw, event))
            {
                event.summary.clone_from(&raw.summary);
            }
        }
    }

    fn prune_transient_utterances(&mut self, simulation: &Simulation) {
        self.transient_utterances.retain(|human_id, events| {
            let Some(human) = simulation.snapshot.human(human_id) else {
                return false;
            };
            events.retain(|raw| {
                human
                    .short_term_memory
                    .iter()
                    .any(|stored| same_utterance_event(raw, stored))
            });
            !events.is_empty()
        });
    }
}

fn same_utterance_event(left: &PerceivedEvent, right: &PerceivedEvent) -> bool {
    left.kind == "utterance"
        && right.kind == "utterance"
        && left.origin_tick == right.origin_tick
        && left.available_at_tick == right.available_at_tick
        && left.source == right.source
}

/// Free-form backend prose is not durable trace data. Redact it before it can
/// enter simulation memory or recorded replay evidence, so replay consumes the
/// same deterministic, non-sensitive value as the original live run.
fn redact_decision_prose(decision: &mut HumanDecision) {
    decision.narrative = REDACTED_DECISION_TEXT.to_string();
    if decision.utterance.is_some() {
        decision.utterance = Some(REDACTED_DECISION_TEXT.to_string());
    }
}

/// Clamp a backend decision into the bounded envelope instead of failing the
/// whole run when it drifts over a limit. Over-limit prose, action counts, or
/// state deltas are non-semantic overruns (the narrative is redacted anyway and
/// the utterance is opaque quoted content), so the deterministic fix is to trim
/// them rather than abort the tick. Semantic problems — unknown commands or
/// unauthorized capabilities — are still fatal, enforced later in the action
/// loop.
fn sanitize_decision(decision: &mut HumanDecision) {
    if decision.actions.len() > MAX_ACTIONS_PER_DECISION {
        decision.actions.truncate(MAX_ACTIONS_PER_DECISION);
    }
    truncate_text_bytes(&mut decision.narrative);
    if let Some(utterance) = decision.utterance.as_mut() {
        truncate_text_bytes(utterance);
    }
    clamp_state_delta(&mut decision.internal_state_delta.stress);
    clamp_state_delta(&mut decision.internal_state_delta.attention);
}

/// Truncate `text` to at most [`MAX_DECISION_TEXT_BYTES`] bytes without slicing
/// a multi-byte UTF-8 character (Chinese narratives overrun the byte budget
/// quickly).
fn truncate_text_bytes(text: &mut String) {
    if text.len() > MAX_DECISION_TEXT_BYTES {
        let boundary = text.floor_char_boundary(MAX_DECISION_TEXT_BYTES);
        text.truncate(boundary);
    }
}

/// Clamp a stress/attention delta into `[-MAX_STATE_DELTA_MAGNITUDE, +..]`,
/// dropping non-finite values so a bad number can never reach state application.
fn clamp_state_delta(delta: &mut Option<f64>) {
    match delta {
        Some(value) if value.is_finite() => {
            *value = value.clamp(-MAX_STATE_DELTA_MAGNITUDE, MAX_STATE_DELTA_MAGNITUDE);
        }
        Some(_) => *delta = None,
        None => {}
    }
}

/// Parse a backend's response text as a [`HumanDecision`]. The backend is
/// expected to return a JSON object (optionally as the entire response text,
/// or embedded as the first `{...}` block). A missing/null/blank narrative is
/// normalized to fixed non-semantic evidence. A malformed action is an output
/// contract failure, not a no-op: retaining it would hide a model tool-use
/// failure from benchmark results. Authorization is still enforced separately
/// by the Action Gateway.
fn parse_decision(text: &str) -> Result<HumanDecision, String> {
    let json_slice = extract_json_object(text)
        .ok_or_else(|| "backend response did not contain a JSON object".to_string())?;
    let mut value: Value = serde_json::from_str(json_slice)
        .map_err(|error| format!("backend response is not valid JSON: {error}"))?;
    normalize_missing_narrative(&mut value)?;
    normalize_actions(&mut value)?;
    let decision: HumanDecision = serde_json::from_value(value)
        .map_err(|error| format!("backend response does not match the decision shape: {error}"))?;
    Ok(decision)
}

/// Reject malformed action proposals. A model occasionally emits a partial
/// entry such as `{"command":"engineShutdown"}`; accepting the remaining
/// output would hide a tool-use failure from the benchmark.
fn normalize_actions(value: &mut Value) -> Result<(), String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| "backend response JSON must be an object".to_string())?;
    let Some(actions) = object.get_mut("actions") else {
        return Ok(());
    };

    let entries = actions
        .as_array()
        .ok_or_else(|| "actions must be an array".to_string())?;
    if entries.iter().any(|entry| {
        let Some(action) = entry.as_object() else {
            return true;
        };
        !matches!(action.get("target"), Some(Value::String(target)) if !target.trim().is_empty())
            || !matches!(action.get("command"), Some(Value::String(command)) if !command.trim().is_empty())
    }) {
        return Err("actions contains an entry without a non-empty target and command".to_string());
    }
    Ok(())
}

fn normalize_missing_narrative(value: &mut Value) -> Result<(), String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| "backend response JSON must be an object".to_string())?;
    let needs_narrative = match object.get("narrative") {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.trim().is_empty(),
        Some(_) => false,
    };
    if needs_narrative {
        // Narrative is redacted before it reaches state or recordings. This
        // preserves a valid no-op/structured decision without inventing any
        // action, utterance, or internal-state change.
        object.insert(
            "narrative".to_string(),
            Value::String(IMPLICIT_NARRATIVE.to_string()),
        );
    }
    Ok(())
}

/// Find the first top-level `{...}` object in `text`, tolerating leading or
/// trailing prose around the JSON block.
///
/// Brace-depth counting must track whether the scanner is currently inside a
/// JSON string literal. A model's `narrative`/`utterance` text is ordinary
/// prose and can legitimately contain a literal `{` or `}` character (for
/// example, describing a labeled control as `the {A} switch`). Without
/// string-awareness, such a character would be counted as a structural brace
/// and could close the object early, truncating the JSON mid-value and
/// causing a spurious parse failure on an otherwise well-formed decision.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Convert an [`HumanTurnError`] into a [`SimulationError`] for callers that
/// need to fail a [`Simulation`] run through the existing error type.
impl From<HumanTurnError> for SimulationError {
    fn from(error: HumanTurnError) -> Self {
        SimulationError::InvalidScenario(error.to_string())
    }
}

/// Test-only public wrapper around [`parse_decision`], so other crates'
/// integration tests can exercise the decision-parsing contract (narrative
/// normalization, tolerance for surrounding prose, JSON shape) without
/// depending on private items.
#[doc(hidden)]
pub fn parse_decision_for_tests(text: &str) -> Result<HumanDecision, String> {
    parse_decision(text)
}

/// Validate a raw backend response before a transport implementation decides
/// whether it needs to request a formatting-only retry.
pub fn validate_decision_output(text: &str) -> Result<(), String> {
    parse_decision(text).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decision_normalizes_missing_narrative() {
        let decision = parse_decision(r#"{"utterance": "hi"}"#).expect("missing narrative is safe");
        assert_eq!(decision.narrative, IMPLICIT_NARRATIVE);
        assert_eq!(decision.utterance.as_deref(), Some("hi"));
    }

    #[test]
    fn parse_decision_normalizes_null_and_blank_narrative() {
        for text in [r#"{"narrative":null}"#, r#"{"narrative":"   "}"#] {
            let decision = parse_decision(text).expect("narrative is normalized");
            assert_eq!(decision.narrative, IMPLICIT_NARRATIVE);
        }
    }

    #[test]
    fn parse_decision_rejects_malformed_action_instead_of_silently_dropping_it() {
        let error = parse_decision(r#"{"actions":[{"command":"engineShutdown"}]}"#)
            .expect_err("partial tool request must be visible as invalid output");
        assert!(error.contains("non-empty target and command"));
    }

    #[test]
    fn parse_decision_accepts_minimal_valid_output() {
        let decision =
            parse_decision(r#"{"narrative": "stayed calm and watched the panel"}"#).expect("ok");
        assert_eq!(decision.narrative, "stayed calm and watched the panel");
        assert!(decision.utterance.is_none());
        assert!(decision.actions.is_empty());
    }

    #[test]
    fn parse_decision_tolerates_surrounding_prose() {
        let decision = parse_decision(
            "Sure, here is my decision:\n{\"narrative\": \"opened the window\"}\nDone.",
        )
        .expect("ok");
        assert_eq!(decision.narrative, "opened the window");
    }

    #[test]
    fn parse_decision_tolerates_literal_braces_inside_string_values() {
        // A narrative or utterance is free-form prose and may legitimately
        // contain a literal brace character, e.g. describing a labeled
        // control. Brace-depth counting must not treat this as a
        // structural JSON delimiter and truncate the object early.
        let decision = parse_decision(
            r#"{"narrative": "pressed the {A} switch", "utterance": "flip the {B} toggle"}"#,
        )
        .expect("a literal brace inside a JSON string value must not truncate the object");
        assert_eq!(decision.narrative, "pressed the {A} switch");
        assert_eq!(decision.utterance.as_deref(), Some("flip the {B} toggle"));
    }

    #[test]
    fn parse_decision_tolerates_escaped_quotes_and_braces_inside_string_values() {
        let decision =
            parse_decision(r#"{"narrative": "the pilot said \"close the {door}\" calmly"}"#)
                .expect("escaped quotes must not desynchronize string-boundary tracking");
        assert_eq!(
            decision.narrative,
            "the pilot said \"close the {door}\" calmly"
        );
    }

    #[test]
    fn parse_decision_rejects_non_json_text() {
        let error = parse_decision("I will open the window.").expect_err("no JSON object");
        assert!(error.contains("JSON object"));
    }

    #[test]
    fn parse_decision_reads_actions_and_delta() {
        let decision = parse_decision(
            r#"{
                "narrative": "felt uneasy and asked for a window",
                "utterance": "could you open a window?",
                "actions": [{"target": "engine-1", "command": "engineShutdown"}],
                "internalStateDelta": {"stress": 0.05, "attention": -0.02}
            }"#,
        )
        .expect("ok");
        assert_eq!(
            decision.utterance,
            Some("could you open a window?".to_string())
        );
        assert_eq!(decision.actions.len(), 1);
        assert_eq!(decision.internal_state_delta.stress, Some(0.05));
        assert_eq!(decision.internal_state_delta.attention, Some(-0.02));
    }

    #[test]
    fn parse_decision_rejects_a_mixed_valid_and_partial_action_list() {
        let error = parse_decision(
            r#"{
                "narrative": "I respond to the alert.",
                "actions": [
                    {"command": "engineShutdown"},
                    {"target": "engine-1", "command": "engineShutdown"},
                    {"target": "hvac-1"}
                ]
            }"#,
        )
        .expect_err("a malformed action must remain visible to the evaluator");
        assert!(error.contains("non-empty target and command"));
    }

    use cockpit_scenario::load_scenario;
    use cockpit_simulation_core::Simulation;

    /// Test backend: the primary human always speaks; everyone reports a fixed
    /// narrative. Deterministic and offline, so it can stand in for a real
    /// backend when exercising the driver.
    struct SpeakingBackend;

    impl HumanBackend for SpeakingBackend {
        async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
            if context.persona.role == "pilot" {
                Ok(
                    r#"{"narrative": "watching the panel", "utterance": "status check"}"#
                        .to_string(),
                )
            } else {
                Ok(r#"{"narrative": "sitting quietly"}"#.to_string())
            }
        }
    }

    #[derive(Default)]
    struct ListeningBackend {
        heard_status_check: bool,
    }

    impl HumanBackend for ListeningBackend {
        async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
            if context.persona.role == "pilot" {
                return Ok(
                    r#"{"narrative": "watching the panel", "utterance": "status check"}"#
                        .to_string(),
                );
            }
            self.heard_status_check |= context
                .delivered_perception
                .iter()
                .any(|event| event.kind == "utterance" && event.summary == "status check");
            Ok(r#"{"narrative": "listening"}"#.to_string())
        }
    }

    /// A backend that always returns malformed output, to exercise the fatal
    /// rejection path.
    struct BrokenBackend;

    impl HumanBackend for BrokenBackend {
        async fn run_turn(&mut self, _context: &HumanTurnContext) -> Result<String, String> {
            Ok(r#"{"internalStateDelta": "not an object"}"#.to_string())
        }
    }

    struct NarrativelessBackend;

    impl HumanBackend for NarrativelessBackend {
        async fn run_turn(&mut self, _context: &HumanTurnContext) -> Result<String, String> {
            Ok(r#"{"utterance":"holding position","actions":[]}"#.to_string())
        }
    }

    struct ActionBackend;

    impl HumanBackend for ActionBackend {
        async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
            if context.persona.role == "pilot" {
                Ok(r#"{"narrative":"activated the alarm","actions":[{"target":"alarm-1","command":"alarmActivate"}]}"#.to_string())
            } else {
                Ok(r#"{"narrative":"watched the cabin"}"#.to_string())
            }
        }
    }

    fn scenario() -> cockpit_simulation_core::SimulationScenario {
        load_scenario("../../scenarios/smoke-in-cockpit.yaml").expect("scenario loads")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recorded_backend_replays_a_live_run_deterministically() {
        // Original run: drive with the speaking backend, collecting per-tick
        // per-human decisions.
        let mut sim = Simulation::new("live-orig", scenario());
        sim.start().expect("starts");
        let mut driver = HumanAgentDriver::new();
        let mut backend = SpeakingBackend;
        let mut recorded: Vec<Vec<HumanTurnEvidence>> = Vec::new();
        let mut last_hash = String::new();
        for _ in 0..12 {
            let (step, humans) = driver
                .step_with_backend(&mut sim, &mut backend)
                .await
                .expect("original tick commits");
            last_hash = step.snapshot_hash.clone();
            recorded.push(humans);
        }

        // Replay: drive a fresh simulation with the recorded decisions only.
        let mut replay_sim = Simulation::new("live-orig", scenario());
        replay_sim.start().expect("starts");
        let mut replay_driver = HumanAgentDriver::new();
        let mut replay_backend = RecordedHumanBackend::from_tick_evidence(&recorded);
        let mut replay_hash = String::new();
        for _ in 0..12 {
            let (step, _humans) = replay_driver
                .step_with_backend(&mut replay_sim, &mut replay_backend)
                .await
                .expect("replay tick commits");
            replay_hash = step.snapshot_hash.clone();
        }

        assert_eq!(
            last_hash, replay_hash,
            "replaying recorded decisions reproduces the original final snapshot hash"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_utterance_reaches_another_human_on_a_later_tick() {
        let mut sim = Simulation::new("social", scenario());
        sim.start().expect("starts");
        let mut driver = HumanAgentDriver::new();
        let mut backend = ListeningBackend::default();

        // The pilot speaks each tick; run enough ticks for the utterance to be
        // delivered to the rear passenger's social perception queue.
        let mut evidence = Vec::new();
        for _ in 0..4 {
            let (_, humans) = driver
                .step_with_backend(&mut sim, &mut backend)
                .await
                .expect("tick commits");
            evidence.extend(humans);
        }

        let passenger = sim
            .snapshot
            .human("rear-passenger-1")
            .expect("passenger exists");
        assert!(
            passenger
                .short_term_memory
                .iter()
                .any(|event| event.kind == "utterance"
                    && event.source == "pilot-1"
                    && event.summary == REDACTED_DECISION_TEXT),
            "the durable world state contains only a redacted utterance marker"
        );
        assert!(
            backend.heard_status_check,
            "the live backend receives the transient original utterance"
        );
        assert!(evidence.iter().all(|turn| {
            turn.decision.narrative == REDACTED_DECISION_TEXT
                && turn
                    .decision
                    .utterance
                    .as_deref()
                    .is_none_or(|utterance| utterance == REDACTED_DECISION_TEXT)
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_backend_returning_invalid_output_aborts_the_tick() {
        let mut sim = Simulation::new("broken", scenario());
        sim.start().expect("starts");
        let tick_before = sim.snapshot.tick;
        let mut driver = HumanAgentDriver::new();
        let mut backend = BrokenBackend;

        let result = driver.step_with_backend(&mut sim, &mut backend).await;
        assert!(
            matches!(result, Err(HumanTurnError::InvalidOutput { .. })),
            "invalid backend output is a fatal error, not a fallback"
        );
        assert_eq!(
            sim.snapshot.tick, tick_before,
            "the tick is not committed when a human's backend turn is invalid"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_backend_missing_only_narrative_commits_the_tick() {
        let mut sim = Simulation::new("narrativeless", scenario());
        sim.start().expect("starts");
        let mut driver = HumanAgentDriver::new();
        let mut backend = NarrativelessBackend;

        let (step, human_turns) = driver
            .step_with_backend(&mut sim, &mut backend)
            .await
            .expect("a structurally valid decision commits without narrative prose");

        assert_eq!(step.tick, 0);
        assert_eq!(sim.snapshot.tick, 1);
        assert_eq!(human_turns.len(), 2);
        assert!(human_turns.iter().all(|turn| {
            turn.decision.narrative == REDACTED_DECISION_TEXT
                && turn.decision.utterance.as_deref() == Some(REDACTED_DECISION_TEXT)
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_human_actions_use_the_configured_agent_grant() {
        let mut sim = Simulation::new("authorized-live-action", scenario());
        sim.start().expect("starts");
        let mut driver = HumanAgentDriver::new();
        let mut backend = ActionBackend;

        let (step, _) = driver
            .step_with_backend(&mut sim, &mut backend)
            .await
            .expect("authorized action commits");

        assert!(step.action_results.iter().any(|result| {
            result.status == cockpit_simulation_core::ActionStatus::Applied
                && result.request.command == Command::AlarmActivate
                && result.request.agent_id == "cockpit-agent"
        }));
    }

    struct UnauthorizedPassengerBackend;

    impl HumanBackend for UnauthorizedPassengerBackend {
        async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
            if context.persona.role == "passenger" {
                return Ok(r#"{"narrative":"tried to operate the engine","actions":[{"target":"engine-1","command":"engineShutdown"}]}"#.to_string());
            }
            Ok(r#"{"narrative":"monitored the cabin"}"#.to_string())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unprivileged_human_cannot_use_the_primary_agent_grant() {
        let mut sim = Simulation::new("unauthorized-human-action", scenario());
        sim.start().expect("starts");
        let mut driver = HumanAgentDriver::new();
        let mut backend = UnauthorizedPassengerBackend;

        let (step, _) = driver
            .step_with_backend(&mut sim, &mut backend)
            .await
            .expect("an unauthorized proposal is dropped, not fatal to the run");
        assert_eq!(
            sim.snapshot.tick, 1,
            "the tick still commits after dropping the unauthorized proposal"
        );
        assert!(
            !step
                .action_results
                .iter()
                .any(|result| result.request.command == Command::EngineShutdown),
            "the passenger's unauthorized engine shutdown is never submitted"
        );
    }

    #[test]
    fn sanitize_decision_trims_over_limit_output_instead_of_failing() {
        let mut decision = HumanDecision {
            utterance: Some("字".repeat(500)),
            actions: vec![
                RequestedAction {
                    target: "alarm-1".to_string(),
                    command: "alarmActivate".to_string(),
                };
                6
            ],
            internal_state_delta: InternalStateDelta {
                stress: Some(0.9),
                attention: Some(f64::NAN),
            },
            narrative: "字".repeat(500),
        };

        sanitize_decision(&mut decision);

        assert_eq!(decision.actions.len(), MAX_ACTIONS_PER_DECISION);
        assert!(decision.narrative.len() <= MAX_DECISION_TEXT_BYTES);
        assert!(
            decision
                .narrative
                .is_char_boundary(decision.narrative.len())
        );
        assert!(decision.utterance.as_ref().unwrap().len() <= MAX_DECISION_TEXT_BYTES);
        assert_eq!(decision.internal_state_delta.stress, Some(0.25));
        assert_eq!(
            decision.internal_state_delta.attention, None,
            "a non-finite delta is dropped rather than applied"
        );
    }

    #[test]
    fn sanitize_decision_leaves_in_bounds_output_untouched() {
        let mut decision = HumanDecision {
            utterance: Some("holding position".to_string()),
            actions: vec![RequestedAction {
                target: "alarm-1".to_string(),
                command: "alarmActivate".to_string(),
            }],
            internal_state_delta: InternalStateDelta {
                stress: Some(0.1),
                attention: Some(-0.05),
            },
            narrative: "watched the cabin".to_string(),
        };
        let expected = decision.clone();

        sanitize_decision(&mut decision);

        assert_eq!(decision, expected);
    }
}
