use super::decision::{
    parse_decision, parse_submitted_decision, parse_turn_output, sanitize_decision,
};
use super::driver::HumanAgentDriver;
use super::types::*;
use super::{
    IMPLICIT_NARRATIVE, MAX_ACTIONS_PER_DECISION, MAX_DECISION_TEXT_BYTES, REDACTED_DECISION_TEXT,
};

#[test]
fn parse_turn_output_accepts_one_tool_call() {
    let HumanTurnOutput::ToolCall(call) = parse_turn_output(
        r#"{"type":"toolCall","tool":"simulation.get_observation","arguments":{}}"#,
    )
    .expect("tool call parses") else {
        panic!("expected tool call");
    };
    assert_eq!(call.tool, "simulation.get_observation");
    assert_eq!(call.arguments, serde_json::json!({}));
}

#[test]
fn parse_submitted_decision_accepts_structured_arguments() {
    let decision = parse_submitted_decision(&serde_json::json!({
        "utterance": "I will stay alert.",
        "internalStateDelta": { "stress": 0.1, "attention": 0.2 },
        "narrative": "I watch the panel and remain ready."
    }))
    .expect("structured decision parses");
    assert_eq!(decision.utterance.as_deref(), Some("I will stay alert."));
    assert_eq!(decision.internal_state_delta.stress, Some(0.1));
    assert!(decision.actions.is_empty());
}

#[test]
fn parse_submitted_decision_rejects_direct_actions() {
    let error = parse_submitted_decision(&serde_json::json!({
        "actions": [{ "target": "engine-1", "command": "engineShutdown" }],
        "narrative": "acting"
    }))
    .expect_err("structured final must not bypass simulation.request_action");
    assert!(error.contains("simulation.request_action"));
}

#[test]
fn parse_turn_output_rejects_actions_in_final() {
    let error = parse_turn_output(
        r#"{"type":"final","actions":[{"target":"engine-1","command":"engineShutdown"}],"narrative":"acting"}"#,
    )
    .expect_err("final must not bypass the action tool");
    assert!(error.contains("simulation.request_action"));
}

#[test]
fn parse_turn_output_accepts_legacy_no_action_final_without_type() {
    let HumanTurnOutput::Final(decision) = parse_turn_output(r#"{"narrative":"kept watch"}"#)
        .expect("a legacy no-action decision is a safe final response")
    else {
        panic!("expected final output");
    };
    assert_eq!(decision.narrative, "kept watch");
}

#[test]
fn parse_turn_output_rejects_legacy_final_with_actions() {
    let error = parse_turn_output(
        r#"{"actions":[{"target":"engine-1","command":"engineShutdown"}],"narrative":"acting"}"#,
    )
    .expect_err("legacy output must not bypass the action tool");
    assert!(error.contains("simulation.request_action"));
}

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
    let decision =
        parse_decision("Sure, here is my decision:\n{\"narrative\": \"opened the window\"}\nDone.")
            .expect("ok");
    assert_eq!(decision.narrative, "opened the window");
}

#[test]
fn parse_decision_skips_non_json_object_before_valid_json() {
    let decision = parse_decision(
        "Example only: {type: final, narrative: invalid}\n{\"narrative\": \"kept watch\"}",
    )
    .expect("a non-JSON example must not hide the valid JSON response");
    assert_eq!(decision.narrative, "kept watch");
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
    let decision = parse_decision(r#"{"narrative": "the pilot said \"close the {door}\" calmly"}"#)
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

use crate::{LocalMcpServer, TOOL_SUBMIT_DECISION, ToolRequest, native_mcp::NativeMcpCall};
use cockpit_scenario::load_scenario;
use cockpit_world::{PerceivedEvent, Simulation};

/// Test backend: the primary human always speaks; everyone reports a fixed
/// narrative. Deterministic and offline, so it can stand in for a real
/// backend when exercising the driver.
struct SpeakingBackend;

impl HumanBackend for SpeakingBackend {
    async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
        if context.persona.role == "pilot" {
            Ok(r#"{"narrative": "watching the panel", "utterance": "status check"}"#.to_string())
        } else {
            Ok(r#"{"narrative": "sitting quietly"}"#.to_string())
        }
    }
}

#[derive(Default)]
struct CountingBackend {
    calls: Vec<(String, u64)>,
}

impl HumanBackend for CountingBackend {
    async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
        self.calls
            .push((context.human_id.clone(), context.round as u64));
        Ok(r#"{"narrative":"monitoring"}"#.to_string())
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
                r#"{"narrative": "watching the panel", "utterance": "status check"}"#.to_string(),
            );
        }
        self.heard_status_check |= context
            .delivered_perception
            .iter()
            .any(|event| event.kind == "utterance" && event.summary == "status check");
        Ok(r#"{"narrative": "listening"}"#.to_string())
    }
}

#[derive(Default)]
struct StructuredDecisionBackend {
    calls: Vec<NativeMcpCall>,
}

impl HumanBackend for StructuredDecisionBackend {
    fn prepare_native_tools(
        &mut self,
        simulation: &Simulation,
        server: &LocalMcpServer,
        context: &HumanTurnContext,
    ) -> Result<(), String> {
        let call_id = format!(
            "structured-{}-{}",
            context.human_id, simulation.snapshot.tick
        );
        let arguments = serde_json::json!({
            "utterance": null,
            "internalStateDelta": { "stress": null, "attention": 0.1 },
            "narrative": "I remain attentive."
        });
        let request = ToolRequest {
            call_id: call_id.clone(),
            run_id: simulation.run_id().to_string(),
            agent_id: simulation.scenario.agent.agent_id.clone(),
            human_id: Some(context.human_id.clone()),
            tick: simulation.snapshot.tick,
            tool_name: TOOL_SUBMIT_DECISION.to_string(),
            arguments: arguments.clone(),
            correlation_id: format!("{call_id}-corr"),
        };
        let mut replay_simulation = simulation.clone();
        let mut replay_server = server.clone();
        let (response, _) = replay_server.call(&mut replay_simulation, request);
        self.calls = vec![NativeMcpCall {
            call_id,
            tool: TOOL_SUBMIT_DECISION.to_string(),
            arguments,
            response,
        }];
        Ok(())
    }

    fn take_native_tool_calls(&mut self) -> Result<Vec<NativeMcpCall>, String> {
        Ok(std::mem::take(&mut self.calls))
    }

    async fn run_turn(&mut self, _context: &HumanTurnContext) -> Result<String, String> {
        Ok("non-JSON ACP transport text that must be ignored".to_string())
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

struct ToolLoopActionBackend;

impl HumanBackend for ToolLoopActionBackend {
    async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
        if context.persona.role != "pilot" {
            return Ok(
                serde_json::json!({ "type": "final", "narrative": "watched the cabin" })
                    .to_string(),
            );
        }
        if context.tool_history.is_empty() {
            return Ok(serde_json::json!({
                "type": "toolCall",
                "tool": "simulation.get_observation",
                "arguments": {}
            })
            .to_string());
        }
        let status = context
            .tool_history
            .iter()
            .find(|exchange| exchange.call.tool == "simulation.get_run_status");
        if status.is_none() {
            return Ok(serde_json::json!({
                "type": "toolCall",
                "tool": "simulation.get_run_status",
                "arguments": {}
            })
            .to_string());
        }
        if !context
            .tool_history
            .iter()
            .any(|exchange| exchange.call.tool == "simulation.request_action")
        {
            let status = &status.expect("status exists").response.result;
            let state_version = status["stateVersion"].as_u64().unwrap_or_default();
            let tick = status["tick"].as_u64().unwrap_or_default();
            return Ok(serde_json::json!({
                "type": "toolCall",
                "tool": "simulation.request_action",
                "arguments": {
                    "target": "alarm-1",
                    "command": "alarmActivate",
                    "expectedStateVersion": state_version,
                    "expiresAtTick": tick + 3
                }
            })
            .to_string());
        }
        Ok(serde_json::json!({
            "type": "final",
            "narrative": "used the alarm action tool"
        })
        .to_string())
    }
}

struct FailsAfterPilotToolBackend;

impl HumanBackend for FailsAfterPilotToolBackend {
    async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
        if context.persona.role == "passenger" {
            return Ok(r#"{"type":"unknown"}"#.to_string());
        }
        let mut delegate = ToolLoopActionBackend;
        delegate.run_turn(context).await
    }
}

fn scenario() -> cockpit_world::SimulationScenario {
    load_scenario("../../scenarios/smoke-in-cockpit.yaml").expect("scenario loads")
}

#[tokio::test(flavor = "current_thread")]
async fn event_driven_tool_schedule_skips_idle_humans_until_cadence() {
    let mut sim = Simulation::new("event-driven-idle", scenario());
    sim.start().expect("starts");
    let mut driver = HumanAgentDriver::new();
    let mut backend = CountingBackend::default();
    let mut server = LocalMcpServer::default();

    let (_, first) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("initial plans run");
    assert_eq!(first.len(), 2);
    assert_eq!(backend.calls.len(), 2);

    for _ in 0..2 {
        let (_, humans) = driver
            .step_with_tools(&mut sim, &mut backend, &mut server)
            .await
            .expect("idle tick commits without a backend call");
        assert!(humans.is_empty());
    }
    assert_eq!(backend.calls.len(), 2);

    let (_, humans) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("cadence tick runs");
    assert_eq!(humans.len(), 2);
    assert_eq!(backend.calls.len(), 4);
}

#[tokio::test(flavor = "current_thread")]
async fn urgent_physical_perception_wakes_only_the_affected_human_before_cadence() {
    let mut sim = Simulation::new("event-driven-urgent", scenario());
    sim.start().expect("starts");
    let mut driver = HumanAgentDriver::new();
    let mut backend = CountingBackend::default();
    let mut server = LocalMcpServer::default();

    driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("initial plans run");
    let tick = sim.snapshot.tick;
    sim.snapshot
        .human_mut("rear-passenger-1")
        .expect("passenger exists")
        .short_term_memory
        .push(PerceivedEvent {
            origin_tick: tick,
            available_at_tick: tick,
            source: "engine-1".to_string(),
            kind: "EngineFire".to_string(),
            summary: "The engine compartment is on fire.".to_string(),
        });

    let (_, humans) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("urgent perception runs");
    assert_eq!(
        humans
            .iter()
            .map(|turn| turn.human_id.as_str())
            .collect::<Vec<_>>(),
        vec!["rear-passenger-1"]
    );
    assert_eq!(backend.calls.len(), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_utterance_waits_for_the_social_reaction_cooldown() {
    let mut sim = Simulation::new("event-driven-social", scenario());
    sim.start().expect("starts");
    let mut driver = HumanAgentDriver::new();
    let mut backend = CountingBackend::default();
    let mut server = LocalMcpServer::default();

    driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("initial plans run");
    let tick = sim.snapshot.tick;
    sim.snapshot
        .human_mut("rear-passenger-1")
        .expect("passenger exists")
        .short_term_memory
        .push(PerceivedEvent {
            origin_tick: tick,
            available_at_tick: tick,
            source: "pilot-1".to_string(),
            kind: "utterance".to_string(),
            summary: "Please check the cabin.".to_string(),
        });

    let (_, humans) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("social cooldown tick commits");
    assert!(humans.is_empty());
    assert_eq!(backend.calls.len(), 2);

    let (_, humans) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("cooled social event runs");
    assert_eq!(
        humans
            .iter()
            .map(|turn| turn.human_id.as_str())
            .collect::<Vec<_>>(),
        vec!["rear-passenger-1"]
    );
    assert_eq!(backend.calls.len(), 3);
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
async fn native_structured_decision_ignores_acp_transport_text() {
    let mut sim = Simulation::new("structured-native", scenario());
    sim.start().expect("starts");
    let mut driver = HumanAgentDriver::new();
    let mut backend = StructuredDecisionBackend::default();
    let mut server = LocalMcpServer::default();

    let (step, evidence) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("structured native decisions commit despite non-JSON ACP text");

    assert_eq!(evidence.len(), 2);
    assert!(evidence.iter().all(|turn| turn.tool_calls.is_empty()));
    assert!(
        evidence
            .iter()
            .all(|turn| turn.decision.internal_state_delta.attention == Some(0.1))
    );
    assert_eq!(
        step.tool_calls
            .iter()
            .filter(|trace| trace.tool_name == TOOL_SUBMIT_DECISION)
            .count(),
        2
    );
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
        result.status == cockpit_world::ActionStatus::Applied
            && result.request.capability_id == "alarm.activate"
            && result.request.agent_id == "cockpit-agent"
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn tool_loop_observes_then_requests_an_action_through_mcp() {
    let mut sim = Simulation::new("tool-driven-live-action", scenario());
    sim.start().expect("starts");
    let mut driver = HumanAgentDriver::new();
    let mut backend = ToolLoopActionBackend;
    let mut server = LocalMcpServer::default();

    let (step, turns) = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await
        .expect("tool-driven action commits");

    assert!(
        step.tool_calls
            .iter()
            .any(|trace| { trace.tool_name == "simulation.get_observation" && trace.allowed })
    );
    assert!(
        step.tool_calls
            .iter()
            .any(|trace| { trace.tool_name == "simulation.request_action" && trace.allowed })
    );
    assert!(step.action_results.iter().any(|result| {
        result.status == cockpit_world::ActionStatus::Applied
            && result.request.capability_id == "alarm.activate"
    }));
    let pilot = turns
        .iter()
        .find(|turn| turn.human_id == "pilot-1")
        .expect("pilot evidence");
    assert_eq!(pilot.tool_calls.len(), 3);
    assert!(
        pilot
            .decision
            .actions
            .iter()
            .any(|action| { action.target == "alarm-1" && action.command == "alarmActivate" })
    );

    let mut replay_sim = Simulation::new("tool-driven-live-action", scenario());
    replay_sim.start().expect("replay starts");
    let mut replay_driver = HumanAgentDriver::new();
    let mut replay_server = LocalMcpServer::default();
    let mut replay_backend = RecordedHumanBackend::from_tick_evidence(&[turns]);
    let (replayed, _) = replay_driver
        .step_with_tools(&mut replay_sim, &mut replay_backend, &mut replay_server)
        .await
        .expect("recorded tool transcript replays");
    assert_eq!(step.snapshot_hash, replayed.snapshot_hash);
    assert_eq!(step.tool_calls.len(), replayed.tool_calls.len());
}

#[tokio::test(flavor = "current_thread")]
async fn failed_tool_loop_does_not_commit_partial_action_state() {
    let mut sim = Simulation::new("transactional-tool-loop", scenario());
    sim.start().expect("starts");
    let mut driver = HumanAgentDriver::new();
    let mut backend = FailsAfterPilotToolBackend;
    let mut server = LocalMcpServer::default();

    let result = driver
        .step_with_tools(&mut sim, &mut backend, &mut server)
        .await;

    assert!(matches!(result, Err(HumanTurnError::InvalidOutput { .. })));
    assert_eq!(sim.snapshot.tick, 0);
    assert!(!sim.snapshot.alarm.active);

    let mut retry_backend = ToolLoopActionBackend;
    let (step, _) = driver
        .step_with_tools(&mut sim, &mut retry_backend, &mut server)
        .await
        .expect("a clean retry commits once");
    assert_eq!(step.action_results.len(), 1);
    assert!(sim.snapshot.alarm.active);
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
            .any(|result| result.request.capability_id == "engine.shutdown"),
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
