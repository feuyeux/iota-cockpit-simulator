//! Per-human, backend-mandatory decision driver.
//!
//! Each tick, every human in the scenario gets one bounded backend/tool loop.
//! The backend (hermes, etc.) starts with persona, needs, delivered perception,
//! and authorized tool schemas but no eager complete observation. Native ACP
//! backends submit the final disposition through `simulation.submit_decision`;
//! synthetic and replay backends retain the bounded textual `toolCall`/`final`
//! compatibility envelope. A final disposition may include:
//! - `internalStateDelta`: bounded numeric adjustments to stress/attention,
//!   applied after range validation.
//! - `utterance`: text enqueued into every other human's social perception
//!   queue for delivery on a later tick (never the same tick).
//! - `narrative`: optional free-form disposition evidence, redacted before it
//!   reaches durable state. Actions are never accepted in `final`; every world
//!   mutation must cross `simulation.request_action` and the Action Gateway.
//!
//! There is no fallback: if any backend round fails, times out, returns malformed
//! output, or exhausts the tool budget, the transactional world/tool copies are
//! discarded and the caller must fail the run.
//!
//! This module is split into cohesive submodules to keep any single file
//! navigable:
//! - [`types`]: wire/decision types, the [`HumanBackend`] trait, and
//!   [`RecordedHumanBackend`].
//! - [`driver`]: [`HumanAgentDriver`], the stateful per-tick orchestration.
//! - [`decision`]: parsing, sanitization, and redaction of raw backend text
//!   into a [`HumanDecision`] or [`HumanTurnOutput`].
//!
//! All previously public items are re-exported here unchanged, so existing
//! `cockpit_agent::live::*` call sites are unaffected by this split.

use crate::{TOOL_ADD_GOAL, TOOL_REQUEST_ACTION, TOOL_WAIT_UNTIL};

mod decision;
mod driver;
mod types;

pub(crate) use decision::parse_submitted_decision;
pub use decision::{parse_decision_for_tests, validate_decision_output, validate_turn_output};
pub use driver::HumanAgentDriver;
pub use types::{
    BackendConversationUpdate, HumanBackend, HumanDecision, HumanToolCall, HumanToolExchange,
    HumanTurnContext, HumanTurnError, HumanTurnEvidence, InternalStateDelta, RecordedHumanBackend,
    RequestedAction,
};

pub(super) const REDACTED_DECISION_TEXT: &str = "[REDACTED]";
pub(super) const IMPLICIT_NARRATIVE: &str = "implicit backend decision";
pub(super) const MAX_ACTIONS_PER_DECISION: usize = 4;
pub(super) const MAX_DECISION_TEXT_BYTES: usize = 1_024;
pub(super) const MAX_STATE_DELTA_MAGNITUDE: f64 = 0.25;
pub const MAX_TOOL_CALLS_PER_TURN: usize = 8;
pub const MAX_TOOL_COST_PER_TURN: u32 = 16;

pub(super) fn tool_call_cost(tool: &str) -> u32 {
    match tool {
        TOOL_REQUEST_ACTION => 4,
        TOOL_ADD_GOAL | TOOL_WAIT_UNTIL => 2,
        _ => 1,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
