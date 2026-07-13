use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    action::{
        ActionRequest, ActionResult, ActionStatus, AgentGrant, Command, ErrorCode, ScriptedAgent,
    },
    clock::{ClockConfig, RunStatus},
    error::{SimulationError, SimulationResult},
    event::{EventEnvelope, EventPayload, ToolCallTrace},
    influence::{ConflictPolicy, InfluenceRule, arbitrate, schedule_due},
    sensor::Observation,
    world::{
        AlarmState, DeviceLifecycle, DeviceState, EnvironmentState, HumanState, WorldSnapshot,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fault {
    pub at_tick: u64,
    pub target: String,
    pub fault_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationScenario {
    pub id: String,
    pub schema_version: u32,
    pub scenario_hash: String,
    pub seed: u64,
    pub clock: ClockConfig,
    pub environment: EnvironmentState,
    pub pilot: HumanState,
    pub engine: DeviceState,
    pub alarm: AlarmState,
    pub faults: Vec<Fault>,
    pub agent: AgentGrant,
    #[serde(default)]
    pub agents: Vec<AgentGrant>,
    pub shutdown_deadline_ticks: u64,
    /// Scheduled, versioned influence rules applied during tick commit. Empty by
    /// default, so scenarios without influences keep identical tick behavior.
    #[serde(default)]
    pub influences: Vec<InfluenceRule>,
    /// Conflict policy used when multiple influence rules target the same
    /// component in one tick.
    #[serde(default = "default_conflict_policy")]
    pub conflict_policy: ConflictPolicy,
}

fn default_conflict_policy() -> ConflictPolicy {
    ConflictPolicy::RejectConflicting
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRecord {
    pub tick: u64,
    pub snapshot_hash: String,
    pub events: Vec<EventEnvelope>,
    pub observation: Observation,
    pub action_results: Vec<ActionResult>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallTrace>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub state_diffs: Vec<StateDiff>,
    #[serde(default)]
    pub plugin_failures: Vec<PluginFailureRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginFailureRecord {
    pub plugin_id: String,
    pub version: String,
    pub reason: String,
    pub decision: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDiff {
    pub source_id: String,
    pub entity_id: String,
    pub component_path: String,
    pub value: Value,
    pub expected_state_version: u64,
}

#[derive(Debug, Clone)]
pub struct Simulation {
    pub scenario: SimulationScenario,
    pub status: RunStatus,
    pub snapshot: WorldSnapshot,
    sequence: u64,
    pending_actions: Vec<ActionRequest>,
    latest_results: Vec<ActionResult>,
}

impl Simulation {
    pub fn new(run_id: impl Into<String>, scenario: SimulationScenario) -> Self {
        let run_id = run_id.into();
        let snapshot = WorldSnapshot {
            run_id,
            tick: 0,
            sim_time_ms: 0,
            version: 0,
            environment: scenario.environment.clone(),
            pilot: scenario.pilot.clone(),
            engine: scenario.engine.clone(),
            alarm: scenario.alarm.clone(),
        };

        Self {
            scenario,
            status: RunStatus::Ready,
            snapshot,
            sequence: 0,
            pending_actions: Vec::new(),
            latest_results: Vec::new(),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.snapshot.run_id
    }

    pub fn observation(&self) -> Observation {
        self.observation_for_agent(&self.scenario.agent.agent_id)
    }

    pub fn observation_for_agent(&self, agent_id: &str) -> Observation {
        Observation::from_snapshot(self.run_id(), agent_id, &self.snapshot)
    }

    pub fn start(&mut self) -> SimulationResult<()> {
        match self.status {
            RunStatus::Ready | RunStatus::Paused | RunStatus::Degraded => {
                self.status = RunStatus::Running;
                Ok(())
            }
            _ => Err(SimulationError::InvalidRunState),
        }
    }

    pub fn pause(&mut self) -> SimulationResult<()> {
        if self.status == RunStatus::Running {
            self.status = RunStatus::Paused;
            Ok(())
        } else {
            Err(SimulationError::InvalidRunState)
        }
    }

    pub fn stop(&mut self) {
        self.status = RunStatus::Stopped;
    }

    pub fn fail(&mut self) {
        self.status = RunStatus::Failed;
    }

    pub fn submit_action(&mut self, request: ActionRequest) -> ActionResult {
        let result = self.validate_action(&request);
        if result.status == ActionStatus::Applied {
            self.pending_actions.push(request);
        }
        result
    }

    pub fn step_with_scripted_agent(
        &mut self,
        agent: &mut ScriptedAgent,
    ) -> SimulationResult<StepRecord> {
        let observation = Observation::from_snapshot(
            self.run_id(),
            &self.scenario.agent.agent_id,
            &self.snapshot,
        );
        for action in agent.next_actions(&observation, self.snapshot.version) {
            self.submit_action(action);
        }
        self.commit_step(observation, Vec::new())
    }

    pub fn step_with_recorded_actions(
        &mut self,
        actions: Vec<ActionRequest>,
    ) -> SimulationResult<StepRecord> {
        self.step_with_recorded_inputs(actions, Vec::new())
    }

    pub fn step_with_recorded_inputs(
        &mut self,
        actions: Vec<ActionRequest>,
        state_diffs: Vec<StateDiff>,
    ) -> SimulationResult<StepRecord> {
        let observation = Observation::from_snapshot(
            self.run_id(),
            &self.scenario.agent.agent_id,
            &self.snapshot,
        );
        for action in actions {
            self.submit_action(action);
        }
        self.commit_step(observation, state_diffs)
    }

    pub fn step_without_agent(&mut self) -> SimulationResult<StepRecord> {
        let observation = Observation::from_snapshot(
            self.run_id(),
            &self.scenario.agent.agent_id,
            &self.snapshot,
        );
        self.commit_step(observation, Vec::new())
    }

    pub fn step_with_state_diffs(&mut self, diffs: Vec<StateDiff>) -> SimulationResult<StepRecord> {
        let observation = Observation::from_snapshot(
            self.run_id(),
            &self.scenario.agent.agent_id,
            &self.snapshot,
        );
        self.commit_step(observation, diffs)
    }

    fn validate_action(&mut self, request: &ActionRequest) -> ActionResult {
        let authorized = if self.scenario.agents.is_empty() {
            self.scenario
                .agent
                .allows(&request.agent_id, &request.command)
        } else {
            self.scenario
                .agents
                .iter()
                .any(|agent| agent.allows(&request.agent_id, &request.command))
        };
        let error_code =
            if !authorized {
                Some(ErrorCode::CapabilityDenied)
            } else if request.expires_at_tick < self.snapshot.tick {
                Some(ErrorCode::ActionExpired)
            } else if request.expected_state_version != self.snapshot.version {
                Some(ErrorCode::VersionMismatch)
            } else if self.pending_actions.iter().any(|pending| {
                pending.target == request.target && pending.command == request.command
            }) {
                Some(ErrorCode::ActionConflict)
            } else {
                match (&request.command, request.target.as_str()) {
                    (Command::EngineShutdown, "engine-1") => {
                        if self.snapshot.engine.power_state != "powered" {
                            Some(ErrorCode::DeviceUnpowered)
                        } else if self.snapshot.engine.shutdown {
                            Some(ErrorCode::PreconditionFailed)
                        } else {
                            None
                        }
                    }
                    (Command::AlarmActivate, "alarm-1") => None,
                    _ => Some(ErrorCode::UnknownTarget),
                }
            };

        let status = if error_code.is_some() {
            ActionStatus::Rejected
        } else {
            ActionStatus::Applied
        };
        let result = ActionResult {
            request: request.clone(),
            status,
            error_code,
            run_id: self.run_id().to_string(),
            tick: self.snapshot.tick,
            correlation_id: request.correlation_id.clone(),
        };
        self.latest_results.push(result.clone());
        result
    }

    fn commit_step(
        &mut self,
        mut observation: Observation,
        state_diffs: Vec<StateDiff>,
    ) -> SimulationResult<StepRecord> {
        if matches!(self.status, RunStatus::Ready | RunStatus::Paused) {
            self.status = RunStatus::Running;
        }
        if self.status != RunStatus::Running && self.status != RunStatus::Degraded {
            return Err(SimulationError::InvalidRunState);
        }

        let tick = self.snapshot.tick;
        let mut events = Vec::new();

        for fault in self.scenario.faults.clone() {
            if fault.at_tick == tick {
                self.apply_fault(&fault, &mut events);
            }
        }

        self.apply_influences(tick, &mut events);
        self.apply_environment(&mut events);
        self.apply_human_influence(&mut events);
        self.apply_pending_actions(&mut events);
        self.apply_state_diffs(&state_diffs, &mut events)?;

        let action_results = std::mem::take(&mut self.latest_results);
        for result in &action_results {
            if result.status == ActionStatus::Rejected {
                let error_code = result
                    .error_code
                    .as_ref()
                    .map(|code| code.stable_code().to_string());
                events.push(self.event_with_error(
                    "ActionRejected",
                    "action-gateway",
                    Some(&result.request.target),
                    error_code,
                    "action rejected by the Action Gateway",
                ));
            }
        }
        observation.action_results = action_results
            .iter()
            .map(|result| format!("{:?}:{}", result.status, result.request.request_id))
            .collect();

        self.snapshot.tick += 1;
        self.snapshot.version += 1;
        self.snapshot.sim_time_ms = self.snapshot.tick * self.scenario.clock.tick_ms;
        let snapshot_hash = self.snapshot.content_hash()?;

        Ok(StepRecord {
            tick,
            snapshot_hash,
            events,
            observation,
            action_results,
            tool_calls: Vec::new(),
            errors: Vec::new(),
            fallback: None,
            state_diffs,
            plugin_failures: Vec::new(),
        })
    }

    fn apply_state_diffs(
        &mut self,
        diffs: &[StateDiff],
        events: &mut Vec<EventEnvelope>,
    ) -> SimulationResult<()> {
        let mut diffs = diffs.to_vec();
        diffs.sort_by(|left, right| {
            left.entity_id
                .cmp(&right.entity_id)
                .then(left.component_path.cmp(&right.component_path))
                .then(left.source_id.cmp(&right.source_id))
        });
        for diff in &diffs {
            if diff.expected_state_version != self.snapshot.version {
                return Err(SimulationError::InvalidScenario(
                    "state diff version does not match the current snapshot".to_string(),
                ));
            }
            validate_state_diff(diff)?;
        }
        for diff in diffs {
            let value = diff.value.as_f64().expect("state diff value is validated");
            write_component_value(
                &mut self.snapshot,
                &diff.entity_id,
                &diff.component_path,
                value,
            );
            events.push(self.event(
                "StateDiffApplied",
                &diff.source_id,
                Some(&diff.entity_id),
                Some(value),
                "validated external state diff applied during tick commit",
            ));
        }
        Ok(())
    }

    /// Apply the scheduled influence rules due this tick under the scenario's
    /// conflict policy. No-op when the scenario declares no influences, so
    /// replay hashes are unchanged for scenarios without influence rules.
    fn apply_influences(&mut self, tick: u64, events: &mut Vec<EventEnvelope>) {
        if self.scenario.influences.is_empty() {
            return;
        }
        let due = schedule_due(&self.scenario.influences, tick);
        if due.is_empty() {
            return;
        }
        let outcome = arbitrate(&due, self.scenario.conflict_policy);

        // Emit a stable rejection event for every rule that lost arbitration.
        for decision in &outcome.decisions {
            if !decision.applied {
                events.push(self.event_with_error(
                    "InfluenceRejected",
                    "influence-system",
                    Some(&decision.entity_id),
                    decision.rejected_reason.clone(),
                    "influence rule rejected by deterministic arbitration",
                ));
            }
        }

        for rule in &outcome.winners {
            let current =
                read_component_value(&self.snapshot, &rule.entity_id, &rule.component_path);
            let target = current.map(|value| rule.op.resolve(value));
            let applied = match target {
                Some(target)
                    if component_value_in_range(&rule.entity_id, &rule.component_path, target) =>
                {
                    write_component_value(
                        &mut self.snapshot,
                        &rule.entity_id,
                        &rule.component_path,
                        target,
                    );
                    events.push(self.event(
                        "InfluenceApplied",
                        "influence-system",
                        Some(&rule.entity_id),
                        Some(target),
                        "scheduled influence rule applied during tick commit",
                    ));
                    true
                }
                _ => false,
            };
            if !applied {
                events.push(self.event_with_error(
                    "InfluenceRejected",
                    "influence-system",
                    Some(&rule.entity_id),
                    Some("influence target is out of range or unknown".to_string()),
                    "influence rule produced an invalid component value",
                ));
            }
        }
    }

    fn apply_fault(&mut self, fault: &Fault, events: &mut Vec<EventEnvelope>) {
        if fault.target == "cabin" && fault.fault_type == "smoke_increase" {
            self.snapshot.environment.fire_active = true;
            self.snapshot.engine.lifecycle = DeviceLifecycle::Warning;
            self.snapshot.engine.faults.push("engine-fire".to_string());
            events.push(self.event(
                "EngineFire",
                "scenario",
                Some("engine-1"),
                Some(self.snapshot.environment.smoke_density),
                "engine fire introduced smoke into cockpit",
            ));
        }
    }

    fn apply_environment(&mut self, events: &mut Vec<EventEnvelope>) {
        let previous_smoke = self.snapshot.environment.smoke_density;
        if self.snapshot.environment.fire_active && !self.snapshot.engine.shutdown {
            self.snapshot.environment.smoke_density =
                (self.snapshot.environment.smoke_density + 0.18).clamp(0.0, 3.0);
        } else {
            self.snapshot.environment.smoke_density =
                (self.snapshot.environment.smoke_density - 0.08).clamp(0.0, 3.0);
        }

        let new_visibility =
            (1.0 / (1.0 + self.snapshot.environment.smoke_density * 1.6)).clamp(0.0, 1.0);
        if (new_visibility - self.snapshot.environment.visibility).abs() > f64::EPSILON {
            self.snapshot.environment.visibility = new_visibility;
            events.push(self.event(
                "VisibilityChanged",
                "environment",
                Some("cabin"),
                Some(new_visibility),
                "smoke changed cockpit visibility",
            ));
        }

        if (self.snapshot.environment.smoke_density - previous_smoke).abs() > f64::EPSILON {
            events.push(self.event(
                "SmokeDensityChanged",
                "environment",
                Some("cabin"),
                Some(self.snapshot.environment.smoke_density),
                "cockpit smoke density changed",
            ));
        }

        if self.snapshot.environment.fire_active && self.snapshot.environment.smoke_density >= 0.18
        {
            events.push(self.event(
                "SmokeDetected",
                "sensor-system",
                Some("cabin"),
                Some(self.snapshot.environment.visibility),
                "perceived smoke risk reached detection threshold",
            ));
        }
    }

    fn apply_human_influence(&mut self, events: &mut Vec<EventEnvelope>) {
        let old_stress = self.snapshot.pilot.stress;
        if self.snapshot.alarm.active || self.snapshot.environment.smoke_density > 0.2 {
            self.snapshot.pilot.stress = (self.snapshot.pilot.stress + 0.04).clamp(0.0, 1.0);
            self.snapshot.pilot.attention = (self.snapshot.pilot.attention - 0.02).clamp(0.0, 1.0);
        }
        if (self.snapshot.pilot.stress - old_stress).abs() > f64::EPSILON {
            events.push(self.event(
                "StressChanged",
                "human-system",
                Some("pilot-1"),
                Some(self.snapshot.pilot.stress),
                "smoke or alarm changed pilot stress",
            ));
        }
    }

    fn apply_pending_actions(&mut self, events: &mut Vec<EventEnvelope>) {
        let mut actions = std::mem::take(&mut self.pending_actions);
        actions.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then(
                    left.command
                        .capability_name()
                        .cmp(right.command.capability_name()),
                )
                .then(left.request_id.cmp(&right.request_id))
        });

        for action in actions {
            match action.command {
                Command::EngineShutdown => {
                    self.snapshot.engine.shutdown = true;
                    self.snapshot.engine.lifecycle = DeviceLifecycle::Recovering;
                    self.snapshot.environment.fire_active = false;
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("engine-1"),
                        None,
                        "engine shutdown action applied",
                    ));
                    events.push(self.event(
                        "EngineShutdown",
                        "device-system",
                        Some("engine-1"),
                        None,
                        "engine shutdown stopped smoke source",
                    ));
                }
                Command::AlarmActivate => {
                    self.snapshot.alarm.active = true;
                    self.snapshot.alarm.volume_db = 85.0;
                    self.snapshot.environment.noise_db = 85.0;
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("alarm-1"),
                        None,
                        "alarm activation action applied",
                    ));
                }
            }
        }
    }

    fn event(
        &mut self,
        event_type: &str,
        source: &str,
        target: Option<&str>,
        value: Option<f64>,
        message: &str,
    ) -> EventEnvelope {
        self.sequence += 1;
        EventEnvelope {
            event_id: format!("{}-evt-{}", self.run_id(), self.sequence),
            event_type: event_type.to_string(),
            run_id: self.run_id().to_string(),
            tick: self.snapshot.tick,
            source: source.to_string(),
            priority: 0,
            sequence: self.sequence,
            correlation_id: format!("{}-corr-{}", self.run_id(), self.sequence),
            payload: EventPayload {
                message: message.to_string(),
                target: target.map(ToString::to_string),
                value,
                error_code: None,
            },
        }
    }

    fn event_with_error(
        &mut self,
        event_type: &str,
        source: &str,
        target: Option<&str>,
        error_code: Option<String>,
        message: &str,
    ) -> EventEnvelope {
        self.sequence += 1;
        EventEnvelope {
            event_id: format!("{}-evt-{}", self.run_id(), self.sequence),
            event_type: event_type.to_string(),
            run_id: self.run_id().to_string(),
            tick: self.snapshot.tick,
            source: source.to_string(),
            priority: 0,
            sequence: self.sequence,
            correlation_id: format!("{}-corr-{}", self.run_id(), self.sequence),
            payload: EventPayload {
                message: message.to_string(),
                target: target.map(ToString::to_string),
                value: None,
                error_code,
            },
        }
    }
}

fn validate_state_diff(diff: &StateDiff) -> SimulationResult<()> {
    let Some(value) = diff.value.as_f64() else {
        return Err(SimulationError::InvalidScenario(
            "state diff value must be numeric".to_string(),
        ));
    };
    component_value_in_range(&diff.entity_id, &diff.component_path, value)
        .then_some(())
        .ok_or_else(|| {
            SimulationError::InvalidScenario(
                "state diff entity, path, or value is invalid".to_string(),
            )
        })
}

/// Whether `value` is within the allowed range for a writable component. Shared
/// by external StateDiff validation and scheduled influence application so both
/// paths honor identical bounds.
fn component_value_in_range(entity_id: &str, component_path: &str, value: f64) -> bool {
    match (entity_id, component_path) {
        ("cabin", "environment.smokeDensity") => (0.0..=3.0).contains(&value),
        ("cabin", "environment.visibility")
        | ("pilot-1", "pilot.stress")
        | ("pilot-1", "pilot.attention")
        | ("engine-1", "engine.health")
        | ("alarm-1", "alarm.active") => (0.0..=1.0).contains(&value),
        ("cabin", "environment.temperatureC") => (-80.0..=100.0).contains(&value),
        _ => false,
    }
}

/// Read the current numeric value of a writable component, if the path is known.
fn read_component_value(
    snapshot: &WorldSnapshot,
    entity_id: &str,
    component_path: &str,
) -> Option<f64> {
    match (entity_id, component_path) {
        ("cabin", "environment.smokeDensity") => Some(snapshot.environment.smoke_density),
        ("cabin", "environment.visibility") => Some(snapshot.environment.visibility),
        ("cabin", "environment.temperatureC") => Some(snapshot.environment.temperature_c),
        ("pilot-1", "pilot.stress") => Some(snapshot.pilot.stress),
        ("pilot-1", "pilot.attention") => Some(snapshot.pilot.attention),
        ("engine-1", "engine.health") => Some(snapshot.engine.health),
        ("alarm-1", "alarm.active") => Some(if snapshot.alarm.active { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Write a validated numeric value to a writable component. The caller must have
/// already confirmed the path is known and the value is in range.
fn write_component_value(
    snapshot: &mut WorldSnapshot,
    entity_id: &str,
    component_path: &str,
    value: f64,
) {
    match (entity_id, component_path) {
        ("cabin", "environment.smokeDensity") => snapshot.environment.smoke_density = value,
        ("cabin", "environment.visibility") => snapshot.environment.visibility = value,
        ("cabin", "environment.temperatureC") => snapshot.environment.temperature_c = value,
        ("pilot-1", "pilot.stress") => snapshot.pilot.stress = value,
        ("pilot-1", "pilot.attention") => snapshot.pilot.attention = value,
        ("engine-1", "engine.health") => snapshot.engine.health = value,
        ("alarm-1", "alarm.active") => snapshot.alarm.active = value > 0.5,
        _ => unreachable!("component path is validated before write"),
    }
}
