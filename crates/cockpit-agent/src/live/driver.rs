//! [`HumanAgentDriver`]: the stateful per-tick orchestration that drives
//! event-triggered backend/tool loops for humans.
//!
//! See the [`super`] module docs for the overall tick/tool-loop contract.

use std::collections::BTreeMap;

use serde_json::Value;

use cockpit_world::{
    ActionRequest, PerceivedEvent,
    perception::{delivered_and_pending, enqueue_social_event, perception_delay_ticks},
    simulation::{HumanStateDelta, Simulation, StepRecord},
};

use crate::{
    LocalMcpServer, OpenWorldControlRequest, TOOL_REQUEST_ACTION, TOOL_SUBMIT_DECISION,
    ToolRequest, open_world::OpenWorldRuntime, redact_json,
};

use super::{
    MAX_ACTIONS_PER_DECISION, MAX_TOOL_CALLS_PER_TURN, MAX_TOOL_COST_PER_TURN,
    REDACTED_DECISION_TEXT,
    decision::{
        parse_decision, parse_submitted_decision, parse_turn_output, redact_decision_prose,
        sanitize_decision,
    },
    tool_call_cost,
    types::{
        HumanBackend, HumanToolCall, HumanToolExchange, HumanTurnContext, HumanTurnError,
        HumanTurnEvidence, HumanTurnOutput, InternalStateDelta, RequestedAction,
    },
};

const MAX_HUMAN_TURN_WALL_TIME_MS: u128 = 120_000;
/// Bound the time an otherwise quiet human can go without reconsidering their
/// situation. Routine sensor deltas are batched until this cadence expires;
/// urgent perception and explicit runtime work wake immediately.
const IDLE_HUMAN_RECHECK_TICKS: u64 = 3;
const SOCIAL_REACTION_COOLDOWN_TICKS: u64 = 2;

fn is_routine_perception(event: &PerceivedEvent) -> bool {
    matches!(
        event.kind.as_str(),
        "SmokeDensityChanged" | "VisibilityChanged" | "CabinTemperatureChanged"
    )
}

fn same_utterance_event(left: &PerceivedEvent, right: &PerceivedEvent) -> bool {
    left.kind == "utterance"
        && right.kind == "utterance"
        && left.origin_tick == right.origin_tick
        && left.available_at_tick == right.available_at_tick
        && left.source == right.source
}

/// Drives one deterministic tick with backend turns only for humans whose
/// event-driven schedule wakes them. Every scheduled decision must come from
/// a real backend call; any failure aborts the tick before state is committed.
#[derive(Clone, Default)]
pub struct HumanAgentDriver {
    /// Raw utterances exist only for the lifetime of a live driver. The world
    /// snapshot keeps redacted markers so recordings and hashes never contain
    /// backend prose, while later live turns still hear what was actually said.
    transient_utterances: BTreeMap<String, Vec<PerceivedEvent>>,
    tool_sequence: u64,
    open_world: OpenWorldRuntime,
}

impl HumanAgentDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_world(&self) -> &OpenWorldRuntime {
        &self.open_world
    }

    pub fn open_world_mut(&mut self) -> &mut OpenWorldRuntime {
        &mut self.open_world
    }

    pub fn sleep_runtime(&self) -> Result<Vec<u8>, String> {
        self.open_world.sleep().map_err(|error| error.to_string())
    }

    pub fn restore_runtime(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.open_world = OpenWorldRuntime::restore(bytes)?;
        Ok(())
    }

    /// Capture or restore a versioned world-plus-agent checkpoint.
    pub fn checkpoint(&self, simulation: &Simulation) -> crate::open_world::OpenWorldCheckpoint {
        crate::open_world::OpenWorldCheckpoint::capture(&simulation.snapshot, &self.open_world)
    }

    pub fn restore_checkpoint(
        &mut self,
        simulation: &mut Simulation,
        bytes: &[u8],
    ) -> Result<(), String> {
        let checkpoint = crate::open_world::OpenWorldCheckpoint::decode(bytes)?;
        if checkpoint.world.run_id != simulation.run_id() {
            return Err("checkpoint runId does not match active simulation".to_string());
        }
        simulation.snapshot = checkpoint.world;
        self.open_world = checkpoint.runtime;
        Ok(())
    }

    /// Drive one tick through an explicit model/tool loop. The simulation,
    /// tool server, transient perception, and open-world control state are
    /// cloned first so a failed later human cannot commit partial world state.
    /// Recovery metadata is retained separately to support deterministic
    /// wait/replan on the next attempt.
    pub async fn step_with_tools<B: HumanBackend>(
        &mut self,
        simulation: &mut Simulation,
        backend: &mut B,
        server: &mut LocalMcpServer,
    ) -> Result<(StepRecord, Vec<HumanTurnEvidence>), HumanTurnError> {
        let mut working_driver = self.clone();
        let mut working_simulation = simulation.clone();
        let mut working_server = server.clone();
        let result = working_driver
            .step_with_tools_inner(&mut working_simulation, backend, &mut working_server)
            .await;
        match result {
            Ok(result) => {
                *self = working_driver;
                *simulation = working_simulation;
                *server = working_server;
                Ok(result)
            }
            Err(error) => {
                let (agent_id, reason) = match &error {
                    HumanTurnError::Backend { human_id, reason }
                    | HumanTurnError::InvalidOutput { human_id, reason } => {
                        (human_id.as_str(), reason.as_str())
                    }
                };
                if agent_id != "simulation" {
                    let tick = simulation.snapshot.tick;
                    working_driver
                        .open_world
                        .record_failure(agent_id, tick, reason);
                    working_driver.open_world.replan(
                        agent_id,
                        tick,
                        "turn transaction rolled back",
                    );
                }
                self.open_world = working_driver.open_world;
                Err(error)
            }
        }
    }

    async fn step_with_tools_inner<B: HumanBackend>(
        &mut self,
        simulation: &mut Simulation,
        backend: &mut B,
        server: &mut LocalMcpServer,
    ) -> Result<(StepRecord, Vec<HumanTurnEvidence>), HumanTurnError> {
        self.prune_transient_utterances(simulation);
        let active_human_ids = simulation
            .snapshot
            .humans
            .iter()
            .map(|human| human.id.clone())
            .collect::<Vec<_>>();
        for human in &simulation.snapshot.humans {
            self.open_world
                .ensure_agent(&human.id, &human.goal, simulation.snapshot.tick);
        }
        let retired = self
            .open_world
            .sessions
            .keys()
            .filter(|id| !active_human_ids.contains(id))
            .cloned()
            .collect::<Vec<_>>();
        for agent_id in retired {
            self.open_world
                .retire_agent(&agent_id, simulation.snapshot.tick);
        }
        let human_ids = self
            .open_world
            .schedule(&active_human_ids, simulation.snapshot.tick);
        let mut evidence = Vec::with_capacity(human_ids.len());
        let mut pending_utterances: Vec<(String, String, String)> = Vec::new();
        let mut deltas: Vec<(String, InternalStateDelta)> = Vec::new();
        let mut traces = Vec::new();

        for human_id in &human_ids {
            let current_tick = simulation.snapshot.tick;
            let mut context = {
                let human = simulation.snapshot.human(human_id).ok_or_else(|| {
                    HumanTurnError::InvalidOutput {
                        human_id: human_id.clone(),
                        reason: "human vanished from the snapshot before its turn".to_string(),
                    }
                })?;
                let (mut delivered, _pending) = delivered_and_pending(human, current_tick);
                self.restore_transient_utterance_text(human_id, &mut delivered);
                let Some(trigger) = self.turn_trigger(human_id, current_tick, &delivered) else {
                    eprintln!(
                        "live human turn skipped: human={human_id} tick={current_tick} reason=idle"
                    );
                    continue;
                };
                eprintln!(
                    "live human turn scheduled: human={human_id} tick={current_tick} trigger={trigger}"
                );
                HumanTurnContext {
                    human_id: human_id.clone(),
                    persona: human.persona.clone(),
                    needs: human.needs,
                    goal: if simulation.scenario.public_goals.is_empty() {
                        human.goal.clone()
                    } else {
                        format!(
                            "{} Public world goals: {}",
                            human.goal,
                            simulation.scenario.public_goals.join("; ")
                        )
                    },
                    delivered_perception: delivered,
                    long_term_memory: {
                        let mut memory = human.long_term_memory.clone();
                        if let Some(session) = self.open_world.sessions.get(human_id) {
                            memory.extend(session.recall(6));
                            if session.backend_session_id.is_none() {
                                memory.extend(session.conversation_recall(6));
                            }
                            if let Some(step) = session.plan.iter().find(|step| {
                                !matches!(
                                    step.status,
                                    crate::open_world::PlanStepStatus::Succeeded
                                        | crate::open_world::PlanStepStatus::Skipped
                                )
                            }) {
                                memory.push(format!(
                                    "active plan [{}]: {}",
                                    step.step_id, step.description
                                ));
                            }
                        }
                        memory
                    },
                    action_capabilities: human.action_capabilities.clone(),
                    tool_history: Vec::new(),
                    round: 0,
                    language: simulation.scenario.language.clone(),
                }
            };
            let turn_started = std::time::Instant::now();
            eprintln!("live human turn start: human={human_id} tick={current_tick}");
            let mut tool_calls = Vec::new();
            let mut tool_cost_spent = 0_u32;
            let mut requested_actions = Vec::new();
            let mut control_requests = Vec::new();

            let mut decision = loop {
                if turn_started.elapsed().as_millis() >= MAX_HUMAN_TURN_WALL_TIME_MS {
                    return Err(HumanTurnError::Backend {
                        human_id: human_id.clone(),
                        reason: format!(
                            "human turn wall-clock budget exceeded ({MAX_HUMAN_TURN_WALL_TIME_MS}ms)"
                        ),
                    });
                }
                context.round = tool_calls.len();
                let backend_started = std::time::Instant::now();
                eprintln!(
                    "live human backend call start: human={human_id} tick={current_tick} round={} tool_calls={} turn_elapsed_ms={}",
                    context.round,
                    tool_calls.len(),
                    turn_started.elapsed().as_millis()
                );
                backend
                    .prepare_native_tools(simulation, server, &context)
                    .map_err(|reason| HumanTurnError::Backend {
                        human_id: human_id.clone(),
                        reason,
                    })?;
                let text = match backend.run_turn(&context).await {
                    Ok(text) => {
                        eprintln!(
                            "live human backend call complete: human={human_id} tick={current_tick} round={} elapsed_ms={} output_bytes={}",
                            context.round,
                            backend_started.elapsed().as_millis(),
                            text.len()
                        );
                        text
                    }
                    Err(reason) => {
                        eprintln!(
                            "live human backend call failed: human={human_id} tick={current_tick} round={} elapsed_ms={} turn_elapsed_ms={} error={reason}",
                            context.round,
                            backend_started.elapsed().as_millis(),
                            turn_started.elapsed().as_millis()
                        );
                        return Err(HumanTurnError::Backend {
                            human_id: human_id.clone(),
                            reason,
                        });
                    }
                };
                if let Some(update) = backend.take_conversation_update() {
                    self.open_world.record_acp_turn(
                        human_id,
                        current_tick,
                        context.round,
                        update.backend,
                        update.backend_session_id,
                        update.response_kind,
                        update.tool_name,
                    );
                }
                let native_calls =
                    backend
                        .take_native_tool_calls()
                        .map_err(|reason| HumanTurnError::Backend {
                            human_id: human_id.clone(),
                            reason,
                        })?;
                if !native_calls.is_empty() {
                    eprintln!(
                        "live human native tools received: human={human_id} tick={current_tick} round={} tool_calls={}",
                        context.round,
                        native_calls.len()
                    );
                }
                let mut submitted_decision = None;
                for native in native_calls {
                    if submitted_decision.is_some() {
                        return Err(HumanTurnError::InvalidOutput {
                            human_id: human_id.clone(),
                            reason: "simulation.submit_decision must be the final native tool call"
                                .to_string(),
                        });
                    }
                    let is_decision_submission = native.tool == TOOL_SUBMIT_DECISION;
                    if !is_decision_submission {
                        if tool_calls.len() >= MAX_TOOL_CALLS_PER_TURN {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: format!(
                                    "tool call budget exceeded ({MAX_TOOL_CALLS_PER_TURN} per turn)"
                                ),
                            });
                        }
                        let cost = tool_call_cost(&native.tool);
                        if tool_cost_spent.saturating_add(cost) > MAX_TOOL_COST_PER_TURN {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: format!(
                                    "tool cost budget exceeded ({MAX_TOOL_COST_PER_TURN} per turn)"
                                ),
                            });
                        }
                        tool_cost_spent += cost;
                        if native.tool == TOOL_REQUEST_ACTION
                            && requested_actions.len() >= MAX_ACTIONS_PER_DECISION
                        {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: format!(
                                    "action tool budget exceeded ({MAX_ACTIONS_PER_DECISION} per turn)"
                                ),
                            });
                        }
                    }
                    let request = ToolRequest {
                        call_id: native.call_id.clone(),
                        run_id: simulation.run_id().to_string(),
                        agent_id: simulation.scenario.agent.agent_id.clone(),
                        human_id: Some(human_id.clone()),
                        tick: simulation.snapshot.tick,
                        tool_name: native.tool.clone(),
                        arguments: native.arguments.clone(),
                        correlation_id: format!("{}-corr", native.call_id),
                    };
                    let (response, trace) = server.call(simulation, request);
                    if response.error.is_some() {
                        self.open_world.record_tool_failure(human_id, &native.tool);
                    }
                    if response != native.response {
                        return Err(HumanTurnError::InvalidOutput {
                            human_id: human_id.clone(),
                            reason: format!(
                                "native MCP result diverged while replaying call {}",
                                native.call_id
                            ),
                        });
                    }
                    traces.push(trace);
                    if is_decision_submission {
                        if response.error.is_some() {
                            eprintln!(
                                "live human structured decision rejected: human={human_id} tick={current_tick} round={} call_id={}",
                                context.round, native.call_id
                            );
                            continue;
                        }
                        submitted_decision =
                            Some(parse_submitted_decision(&native.arguments).map_err(
                                |reason| HumanTurnError::InvalidOutput {
                                    human_id: human_id.clone(),
                                    reason,
                                },
                            )?);
                        continue;
                    }
                    control_requests.extend(server.take_control_requests());
                    if native.tool == TOOL_REQUEST_ACTION
                        && let (Some(target), Some(command)) = (
                            native.arguments.get("target").and_then(Value::as_str),
                            native.arguments.get("command").and_then(Value::as_str),
                        )
                    {
                        requested_actions.push(RequestedAction {
                            target: target.to_string(),
                            command: command.to_string(),
                        });
                    }
                    let recorded_call = HumanToolCall {
                        tool: native.tool,
                        arguments: redact_json(native.arguments),
                    };
                    context.tool_history.push(HumanToolExchange {
                        call_id: native.call_id,
                        call: recorded_call.clone(),
                        response,
                    });
                    tool_calls.push(recorded_call);
                }
                if let Some(decision) = submitted_decision {
                    eprintln!(
                        "live human structured decision accepted: human={human_id} tick={current_tick} round={} source=native_mcp",
                        context.round
                    );
                    break decision;
                }
                match parse_turn_output(&text).map_err(|reason| HumanTurnError::InvalidOutput {
                    human_id: human_id.clone(),
                    reason,
                })? {
                    HumanTurnOutput::Final(decision) => {
                        eprintln!(
                            "live human structured decision accepted: human={human_id} tick={current_tick} round={} source=acp_text_fallback",
                            context.round
                        );
                        break decision;
                    }
                    HumanTurnOutput::ToolCall(call) => {
                        if tool_calls.len() >= MAX_TOOL_CALLS_PER_TURN {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: format!(
                                    "tool call budget exceeded ({MAX_TOOL_CALLS_PER_TURN} per turn)"
                                ),
                            });
                        }
                        let cost = tool_call_cost(&call.tool);
                        if tool_cost_spent.saturating_add(cost) > MAX_TOOL_COST_PER_TURN {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: format!(
                                    "tool cost budget exceeded ({MAX_TOOL_COST_PER_TURN} per turn)"
                                ),
                            });
                        }
                        tool_cost_spent += cost;
                        if call.tool == TOOL_REQUEST_ACTION
                            && requested_actions.len() >= MAX_ACTIONS_PER_DECISION
                        {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: format!(
                                    "action tool budget exceeded ({MAX_ACTIONS_PER_DECISION} per turn)"
                                ),
                            });
                        }

                        self.tool_sequence += 1;
                        let call_id = format!(
                            "{}-human-{}-tick-{}-tool-{}",
                            simulation.run_id(),
                            human_id,
                            simulation.snapshot.tick,
                            self.tool_sequence
                        );
                        let correlation_id = format!("{call_id}-corr");
                        let request = ToolRequest {
                            call_id: call_id.clone(),
                            run_id: simulation.run_id().to_string(),
                            agent_id: simulation.scenario.agent.agent_id.clone(),
                            human_id: Some(human_id.clone()),
                            tick: simulation.snapshot.tick,
                            tool_name: call.tool.clone(),
                            arguments: call.arguments.clone(),
                            correlation_id,
                        };
                        let (response, trace) = server.call(simulation, request);
                        control_requests.extend(server.take_control_requests());
                        if response.error.is_some() {
                            self.open_world.record_tool_failure(human_id, &call.tool);
                        }
                        if call.tool == TOOL_REQUEST_ACTION
                            && let (Some(target), Some(command)) = (
                                call.arguments.get("target").and_then(Value::as_str),
                                call.arguments.get("command").and_then(Value::as_str),
                            )
                        {
                            requested_actions.push(RequestedAction {
                                target: target.to_string(),
                                command: command.to_string(),
                            });
                        }
                        let recorded_call = HumanToolCall {
                            tool: call.tool,
                            arguments: redact_json(call.arguments),
                        };
                        context.tool_history.push(HumanToolExchange {
                            call_id,
                            call: recorded_call.clone(),
                            response,
                        });
                        tool_calls.push(recorded_call);
                        traces.push(trace);
                    }
                }
            };

            if !decision.actions.is_empty() {
                return Err(HumanTurnError::InvalidOutput {
                    human_id: human_id.clone(),
                    reason: "final output must not contain actions; use simulation.request_action"
                        .to_string(),
                });
            }
            decision.actions = requested_actions;
            sanitize_decision(&mut decision);
            eprintln!(
                "live human turn complete: human={human_id} tick={current_tick} rounds={} tool_calls={} elapsed_ms={}",
                context.round + 1,
                tool_calls.len(),
                turn_started.elapsed().as_millis()
            );
            let transient_utterance = decision.utterance.clone();
            redact_decision_prose(&mut decision);
            let location = simulation
                .snapshot
                .human(human_id)
                .map(|human| human.location.clone())
                .unwrap_or_default();
            if let Some(utterance) = transient_utterance {
                pending_utterances.push((human_id.clone(), location, utterance));
            }
            deltas.push((human_id.clone(), decision.internal_state_delta));
            let related_agents = active_human_ids
                .iter()
                .filter(|other| *other != human_id)
                .cloned()
                .collect::<Vec<_>>();
            self.open_world.record_turn(
                human_id,
                current_tick,
                &decision,
                &tool_calls,
                &related_agents,
            );
            for control in control_requests {
                match control {
                    OpenWorldControlRequest::AddGoal {
                        human_id: owner,
                        description,
                        priority,
                    } => {
                        if owner != *human_id
                            || self
                                .open_world
                                .add_goal(&owner, description, priority, current_tick)
                                .is_none()
                        {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: "open-world goal control could not be committed"
                                    .to_string(),
                            });
                        }
                    }
                    OpenWorldControlRequest::WaitUntil {
                        human_id: owner,
                        wake_tick,
                    } => {
                        if owner != *human_id {
                            return Err(HumanTurnError::InvalidOutput {
                                human_id: human_id.clone(),
                                reason: "open-world wait control owner mismatch".to_string(),
                            });
                        }
                        self.open_world.wait_until(&owner, wake_tick);
                    }
                }
            }
            evidence.push(HumanTurnEvidence {
                human_id: human_id.clone(),
                decision,
                tool_calls,
                latency_ms: Some(turn_started.elapsed().as_millis() as u64),
            });
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
        let mut step =
            simulation
                .step_without_agent()
                .map_err(|error| HumanTurnError::InvalidOutput {
                    human_id: "simulation".to_string(),
                    reason: error.to_string(),
                })?;
        step.tool_calls = traces;
        Ok((step, evidence))
    }

    fn turn_trigger(
        &self,
        human_id: &str,
        current_tick: u64,
        delivered: &[PerceivedEvent],
    ) -> Option<String> {
        let session = self.open_world.sessions.get(human_id)?;
        if matches!(
            session.lifecycle,
            crate::open_world::AgentLifecycle::Recovering
        ) {
            return Some("recovery".to_string());
        }
        if session.plan.iter().any(|step| {
            matches!(
                step.status,
                crate::open_world::PlanStepStatus::Pending
                    | crate::open_world::PlanStepStatus::Running
            ) && step
                .retry_after_tick
                .is_none_or(|retry| retry <= current_tick)
        }) {
            return Some("pending-plan".to_string());
        }
        // A failed turn transaction leaves the simulation tick unchanged while
        // preserving runtime recovery metadata. Humans that already completed
        // a turn in that uncommitted tick must be eligible for the clean retry.
        if session.last_active_tick == current_tick {
            return Some("transaction-retry".to_string());
        }
        if let Some(event) = delivered.iter().find(|event| {
            if event.available_at_tick <= session.last_active_tick || is_routine_perception(event) {
                return false;
            }
            event.kind != "utterance"
                || current_tick.saturating_sub(session.last_active_tick)
                    >= SOCIAL_REACTION_COOLDOWN_TICKS
        }) {
            return Some(format!(
                "urgent-perception kind={} source={}",
                event.kind, event.source
            ));
        }
        (current_tick.saturating_sub(session.last_active_tick) >= IDLE_HUMAN_RECHECK_TICKS)
            .then_some("cadence".to_string())
    }

    /// Legacy helper: run one direct decision per human via `backend`, apply their
    /// decisions, then commit the deterministic tick. This compatibility path
    /// invokes the backend once per human without tools; production Live runs
    /// use [`Self::step_with_tools`].
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
                    persona: human.persona.clone(),
                    needs: human.needs,
                    goal: if simulation.scenario.public_goals.is_empty() {
                        human.goal.clone()
                    } else {
                        format!(
                            "{} Public world goals: {}",
                            human.goal,
                            simulation.scenario.public_goals.join("; ")
                        )
                    },
                    delivered_perception: delivered,
                    long_term_memory: {
                        let mut memory = human.long_term_memory.clone();
                        if let Some(session) = self.open_world.sessions.get(human_id) {
                            memory.extend(session.recall(20));
                            if session.backend_session_id.is_none() {
                                memory.extend(session.conversation_recall(20));
                            }
                            if let Some(step) = session.plan.iter().find(|step| {
                                !matches!(
                                    step.status,
                                    crate::open_world::PlanStepStatus::Succeeded
                                        | crate::open_world::PlanStepStatus::Skipped
                                )
                            }) {
                                memory.push(format!(
                                    "active plan [{}]: {}",
                                    step.step_id, step.description
                                ));
                            }
                        }
                        memory
                    },
                    action_capabilities: human.action_capabilities.clone(),
                    tool_history: Vec::new(),
                    round: 0,
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
            if let Some(update) = backend.take_conversation_update() {
                self.open_world.record_acp_turn(
                    human_id,
                    current_tick,
                    context.round,
                    update.backend,
                    update.backend_session_id,
                    update.response_kind,
                    update.tool_name,
                );
            }

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
                let Some(capability) = simulation.capabilities().get_by_wire_name(&action.command)
                else {
                    continue;
                };
                let capability_id = capability.id.clone();
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
                    .any(|owned| owned == &capability_id)
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
                    capability_id,
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
                tool_calls: Vec::new(),
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
