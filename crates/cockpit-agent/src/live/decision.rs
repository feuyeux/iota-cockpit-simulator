//! Parsing, sanitization, and redaction of raw backend text into a
//! [`HumanDecision`] or [`HumanTurnOutput`].
//!
//! See the [`super`] module docs for the overall tick/tool-loop contract.

use cockpit_world::error::SimulationError;
use serde_json::Value;

use super::{
    IMPLICIT_NARRATIVE, MAX_ACTIONS_PER_DECISION, MAX_DECISION_TEXT_BYTES,
    MAX_STATE_DELTA_MAGNITUDE, REDACTED_DECISION_TEXT,
    types::{HumanDecision, HumanToolCall, HumanTurnError, HumanTurnOutput},
};

/// Free-form backend prose is not durable trace data. Redact it before it can
/// enter simulation memory or recorded replay evidence, so replay consumes the
/// same deterministic, non-sensitive value as the original live run.
pub(super) fn redact_decision_prose(decision: &mut HumanDecision) {
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
pub(super) fn sanitize_decision(decision: &mut HumanDecision) {
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

/// Parse one backend round. A round either requests exactly one simulation
/// tool or ends the person's tick with a final disposition. Direct actions in
/// the final object are rejected so every world mutation crosses the audited
/// `simulation.request_action` boundary.
pub(super) fn parse_turn_output(text: &str) -> Result<HumanTurnOutput, String> {
    let mut value = parse_json_object(text)?;
    let output_type = {
        let object = value
            .as_object_mut()
            .ok_or_else(|| "backend response JSON must be an object".to_string())?;
        object
            .remove("type")
            .and_then(|value| value.as_str().map(str::to_string))
    };
    match output_type.as_deref() {
        Some("toolCall") => {
            let object = value
                .as_object_mut()
                .expect("value was verified as an object above");
            let tool = object
                .remove("tool")
                .and_then(|value| value.as_str().map(str::to_string))
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "toolCall must contain a non-empty tool".to_string())?;
            let arguments = object
                .remove("arguments")
                .unwrap_or_else(|| serde_json::json!({}));
            if !arguments.is_object() {
                return Err("toolCall arguments must be an object".to_string());
            }
            Ok(HumanTurnOutput::ToolCall(HumanToolCall { tool, arguments }))
        }
        Some("final") | None => parse_final_output(value),
        Some(_) => Err("backend response type must be toolCall or final".to_string()),
    }
}

/// Older ACP agents may emit a direct no-action decision without the tool-loop
/// `type` discriminator. Treat it as a final response only after enforcing the
/// same action prohibition as an explicit `final` object.
fn parse_final_output(value: Value) -> Result<HumanTurnOutput, String> {
    parse_final_decision(value).map(HumanTurnOutput::Final)
}

pub(crate) fn parse_submitted_decision(arguments: &Value) -> Result<HumanDecision, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "simulation.submit_decision arguments must be an object".to_string())?;
    if object.contains_key("actions") {
        return Err(
            "simulation.submit_decision must not contain actions; use simulation.request_action"
                .to_string(),
        );
    }
    if let Some(field) = object.keys().find(|field| {
        !matches!(
            field.as_str(),
            "utterance" | "internalStateDelta" | "narrative"
        )
    }) {
        return Err(format!(
            "simulation.submit_decision contains unknown field {field}"
        ));
    }
    if let Some(delta) = object.get("internalStateDelta") {
        let delta = delta.as_object().ok_or_else(|| {
            "simulation.submit_decision internalStateDelta must be an object".to_string()
        })?;
        if let Some(field) = delta
            .keys()
            .find(|field| !matches!(field.as_str(), "stress" | "attention"))
        {
            return Err(format!(
                "simulation.submit_decision internalStateDelta contains unknown field {field}"
            ));
        }
    }
    parse_final_decision(arguments.clone())
        .map_err(|error| format!("simulation.submit_decision arguments are invalid: {error}"))
}

fn parse_final_decision(mut value: Value) -> Result<HumanDecision, String> {
    normalize_missing_narrative(&mut value)?;
    normalize_actions(&mut value)?;
    let decision: HumanDecision = serde_json::from_value(value).map_err(|error| {
        format!("backend final output does not match the decision shape: {error}")
    })?;
    if !decision.actions.is_empty() {
        return Err(
            "final output must not contain actions; use simulation.request_action".to_string(),
        );
    }
    Ok(decision)
}

/// Parse a backend's response text as a [`HumanDecision`]. The backend is
/// expected to return a JSON object (optionally as the entire response text,
/// or embedded as the first `{...}` block). A missing/null/blank narrative is
/// normalized to fixed non-semantic evidence. A malformed action is an output
/// contract failure, not a no-op: retaining it would hide a model tool-use
/// failure from benchmark results. Authorization is still enforced separately
/// by the Action Gateway.
pub(super) fn parse_decision(text: &str) -> Result<HumanDecision, String> {
    let mut value = parse_json_object(text)?;
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

/// Parse the first valid JSON object in `text`, tolerating leading/trailing
/// prose and non-JSON brace-delimited examples before the actual response.
fn parse_json_object(text: &str) -> Result<Value, String> {
    let candidates = extract_json_objects(text);
    if candidates.is_empty() {
        return Err("backend response did not contain a JSON object".to_string());
    }

    let mut last_error = None;
    for candidate in candidates {
        match serde_json::from_str(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "backend response is not valid JSON: {}",
        last_error.expect("a non-empty candidate list must produce a parse error")
    ))
}

/// Find brace-balanced `{...}` objects in `text`.
///
/// Brace-depth counting must track whether the scanner is currently inside a
/// JSON string literal. A model's `narrative`/`utterance` text is ordinary
/// prose and can legitimately contain a literal `{` or `}` character (for
/// example, describing a labeled control as `the {A} switch`). Without
/// string-awareness, such a character would be counted as a structural brace
/// and could close the object early, truncating the JSON mid-value and
/// causing a spurious parse failure on an otherwise well-formed decision.
fn extract_json_objects(text: &str) -> Vec<&str> {
    text.char_indices()
        .filter_map(|(start, ch)| (ch == '{').then_some(start))
        .filter_map(|start| {
            extract_balanced_object(&text[start..]).map(|end| &text[start..start + end])
        })
        .collect()
}

fn extract_balanced_object(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text.char_indices() {
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
                    return Some(offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// Convert an [`HumanTurnError`] into a [`SimulationError`] for callers that
/// need to fail a [`cockpit_world::simulation::Simulation`] run
/// through the existing error type.
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

/// Validate one raw tool-loop backend response before a transport requests a
/// formatting-only retry.
pub fn validate_turn_output(text: &str) -> Result<(), String> {
    parse_turn_output(text).map(|_| ())
}

/// Validate a legacy direct decision response used by compatibility callers.
pub fn validate_decision_output(text: &str) -> Result<(), String> {
    parse_decision(text).map(|_| ())
}
