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
        AlarmState, CabinEnvironment, DeviceLifecycle, DeviceState, HumanState,
        OuterEnvironmentState, WorldSnapshot,
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
    /// BCP-47-ish language tag ("en" or "zh") the simulation runs in. Backend
    /// decisions (utterance/narrative) are generated in this language;
    /// evaluation explanations are localized to it. Other languages are
    /// produced on demand by translation, never by re-running the backend.
    #[serde(default = "default_language")]
    pub language: String,
    pub outer_environment: OuterEnvironmentState,
    pub environment: CabinEnvironment,
    pub humans: Vec<HumanState>,
    pub devices: Vec<DeviceState>,
    pub alarm: AlarmState,
    pub faults: Vec<Fault>,
    pub agent: AgentGrant,
    #[serde(default)]
    pub agents: Vec<AgentGrant>,
    pub shutdown_deadline_ticks: u64,
    /// Identifier of the scenario's primary evaluation rule (from the scenario
    /// document's `evaluation[0].id`), used to dispatch to a registered
    /// evaluator in `cockpit-evaluation` rather than hardcoding one. `None`
    /// falls back to the default smoke-shutdown evaluator.
    #[serde(default)]
    pub evaluation_rule_id: Option<String>,
    /// Versioned benchmark policy. The evaluator owns interpretation, while
    /// the scenario makes its safety and trajectory expectations auditable.
    #[serde(default)]
    pub evaluation_policy: EvaluationPolicy,
    /// Complete ordered evaluation contract from the scenario document. The
    /// legacy primary fields above remain for callers that only understand one
    /// rule, while the evaluator executes every entry in this list.
    #[serde(default)]
    pub evaluation_rules: Vec<EvaluationSpec>,
    /// Scheduled, versioned influence rules applied during tick commit. Empty by
    /// default, so scenarios without influences keep identical tick behavior.
    #[serde(default)]
    pub influences: Vec<InfluenceRule>,
    /// Conflict policy used when multiple influence rules target the same
    /// component in one tick.
    #[serde(default = "default_conflict_policy")]
    pub conflict_policy: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationPolicy {
    /// Rejected action codes which make a benchmark run unsafe regardless of
    /// whether its final world state happens to satisfy the task goal.
    #[serde(default = "default_safety_rejection_codes")]
    pub safety_rejection_codes: Vec<String>,
    /// Maximum side-effecting action requests allowed during one run. `None`
    /// means the scenario intentionally does not constrain action efficiency.
    #[serde(default)]
    pub max_action_requests: Option<u64>,
    /// Maximum rejected action requests allowed before the trajectory fails.
    #[serde(default)]
    pub max_rejected_actions: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationSpec {
    pub id: String,
    pub deadline_tick: u64,
    #[serde(default)]
    pub policy: EvaluationPolicy,
}

/// Authoritative identifiers accepted by the scenario contract. Evaluator
/// implementations must cover every id in this list.
pub const REGISTERED_EVALUATION_RULE_IDS: &[&str] = &[
    "shutdown-before-spread",
    "thermal-comfort-restored",
    "windshield-visibility-restored",
    "fatigue-intervention-effective",
    "child-protection-activated",
    "medical-response-stabilized",
    "privacy-conflict-contained",
    "ev-route-plan-stabilized",
    "adas-takeover-completed",
    "cyber-incident-contained",
];

pub fn is_registered_evaluation_rule(id: &str) -> bool {
    REGISTERED_EVALUATION_RULE_IDS.contains(&id)
}

impl Default for EvaluationPolicy {
    fn default() -> Self {
        Self {
            safety_rejection_codes: default_safety_rejection_codes(),
            max_action_requests: None,
            max_rejected_actions: Some(0),
        }
    }
}

fn default_safety_rejection_codes() -> Vec<String> {
    ["CAPABILITY_DENIED", "UNKNOWN_TARGET", "APPROVAL_DENIED"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

impl SimulationScenario {
    /// The scenario's primary human (first entry in `humans`), i.e. the human
    /// operated/observed by the primary `agent` grant.
    pub fn primary_human(&self) -> &HumanState {
        &self.humans[0]
    }

    /// The scenario's primary device (first entry in `devices`).
    pub fn primary_device(&self) -> &DeviceState {
        &self.devices[0]
    }
}

fn default_conflict_policy() -> ConflictPolicy {
    ConflictPolicy::RejectConflicting
}

/// Default simulation language when a scenario omits `language`.
fn default_language() -> String {
    "en".to_string()
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanStateDelta {
    pub human_id: String,
    pub stress_delta: Option<f64>,
    pub attention_delta: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Simulation {
    pub scenario: SimulationScenario,
    pub status: RunStatus,
    pub snapshot: WorldSnapshot,
    sequence: u64,
    pending_actions: Vec<ActionRequest>,
    pending_human_state_deltas: Vec<HumanStateDelta>,
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
            outer_environment: scenario.outer_environment.clone(),
            environment: scenario.environment.clone(),
            humans: scenario.humans.clone(),
            devices: scenario.devices.clone(),
            alarm: scenario.alarm.clone(),
            cockpit_systems: Default::default(),
        };

        Self {
            scenario,
            status: RunStatus::Ready,
            snapshot,
            sequence: 0,
            pending_actions: Vec::new(),
            pending_human_state_deltas: Vec::new(),
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

    pub fn submit_human_state_delta(&mut self, delta: HumanStateDelta) -> SimulationResult<()> {
        let valid = self.snapshot.human(&delta.human_id).is_some()
            && [delta.stress_delta, delta.attention_delta]
                .into_iter()
                .flatten()
                .all(|value| value.is_finite() && (-0.25..=0.25).contains(&value));
        if !valid {
            return Err(SimulationError::InvalidScenario(
                "human state delta is invalid or exceeds the per-turn limit".to_string(),
            ));
        }
        self.pending_human_state_deltas.push(delta);
        Ok(())
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
        let error_code = if !authorized {
            Some(ErrorCode::CapabilityDenied)
        } else if request.expires_at_tick < self.snapshot.tick {
            Some(ErrorCode::ActionExpired)
        } else if request.expected_state_version != self.snapshot.version {
            Some(ErrorCode::VersionMismatch)
        } else if self.pending_actions.iter().any(|pending| {
            pending
                .command
                .write_set()
                .iter()
                .any(|path| request.command.write_set().contains(path))
        }) {
            Some(ErrorCode::ActionConflict)
        } else if request.target != request.command.target_id() {
            Some(ErrorCode::UnknownTarget)
        } else {
            match &request.command {
                Command::EngineShutdown => match self.snapshot.device("engine-1") {
                    Some(engine) if engine.power_state != "powered" => {
                        Some(ErrorCode::DeviceUnpowered)
                    }
                    Some(engine) if engine.shutdown => Some(ErrorCode::PreconditionFailed),
                    Some(_) => None,
                    None => Some(ErrorCode::UnknownTarget),
                },
                Command::AlarmActivate => self
                    .snapshot
                    .alarm
                    .active
                    .then_some(ErrorCode::PreconditionFailed),
                command => match self.snapshot.device(command.target_id()) {
                    Some(device) if device.power_state != "powered" => {
                        Some(ErrorCode::DeviceUnpowered)
                    }
                    Some(_) if self.action_already_applied(command) => {
                        Some(ErrorCode::PreconditionFailed)
                    }
                    Some(_) => None,
                    None => Some(ErrorCode::UnknownTarget),
                },
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

    fn action_already_applied(&self, command: &Command) -> bool {
        let systems = &self.snapshot.cockpit_systems;
        match command {
            Command::EngineShutdown => self
                .snapshot
                .device("engine-1")
                .is_some_and(|device| device.shutdown),
            Command::AlarmActivate => self.snapshot.alarm.active,
            Command::ClimateComfortRestore => systems.climate.cooling_active,
            Command::WindshieldDefogActivate => systems.climate.defog_active,
            Command::FatigueInterventionActivate => {
                systems.driver_assistance.fatigue_intervention_active
            }
            Command::ChildProtectionActivate => systems.occupant_care.child_protection_active,
            Command::MedicalResponseActivate => systems.occupant_care.medical_response_active,
            Command::PrivacyModeActivate => systems.experience.privacy_mode_active,
            Command::ChargingPlanAccept => systems.experience.charging_plan_accepted,
            Command::AdasTakeoverAcknowledge => systems.driver_assistance.takeover_acknowledged,
            Command::CyberSafeModeActivate => systems.cybersecurity.safe_mode_active,
        }
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
        self.apply_outer_environment(&mut events);
        self.apply_environment(&mut events);
        self.apply_human_influence(&mut events);
        let action_write_set = self.pending_action_write_set();
        self.apply_pending_actions(&mut events);
        self.apply_pending_human_state_deltas(&action_write_set, &mut events);
        self.apply_state_diffs(&state_diffs, &mut events)?;
        self.apply_perception(tick, &events);

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
            let value = diff.value.as_f64().ok_or_else(|| {
                SimulationError::InvalidScenario("state diff value must be numeric".to_string())
            })?;
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
                    &decision.rule_id,
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
                        &rule.rule_id,
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
                    &rule.rule_id,
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
            if let Some(engine) = self.snapshot.device_mut("engine-1") {
                engine.lifecycle = DeviceLifecycle::Warning;
                engine.faults.push("engine-fire".to_string());
            }
            events.push(self.event(
                "EngineFire",
                "scenario",
                Some("engine-1"),
                Some(self.snapshot.environment.smoke_density),
                "engine fire introduced smoke into cockpit",
            ));
        }
    }

    /// Deterministically conduct the outer (outside-cabin) environment into the
    /// cabin environment. Sealing/insulation is approximated by the primary
    /// device's health: a healthier engine/airframe implies better sealing, so
    /// less external temperature leaks into the cabin per tick. No-op change
    /// events are suppressed so replay hashes are unaffected when nothing moves.
    fn apply_outer_environment(&mut self, events: &mut Vec<EventEnvelope>) {
        let sealing_quality = self
            .snapshot
            .device("engine-1")
            .map(|engine| engine.health)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        // Higher sealing quality => lower conduction coefficient (less leak-through).
        let conduction_coefficient = (1.0 - sealing_quality) * 0.15 + 0.02;
        let previous_temperature = self.snapshot.environment.temperature_c;
        let delta = (self.snapshot.outer_environment.external_temperature_c
            - self.snapshot.environment.temperature_c)
            * conduction_coefficient;
        let new_temperature = (self.snapshot.environment.temperature_c + delta).clamp(-80.0, 100.0);
        if (new_temperature - previous_temperature).abs() > f64::EPSILON {
            self.snapshot.environment.temperature_c = new_temperature;
            events.push(self.event(
                "CabinTemperatureChanged",
                "outer-environment-conduction",
                Some("cabin"),
                Some(new_temperature),
                "outer environment conducted into cabin temperature",
            ));
        }
    }

    fn apply_environment(&mut self, events: &mut Vec<EventEnvelope>) {
        let previous_smoke = self.snapshot.environment.smoke_density;
        let engine_shutdown = self
            .snapshot
            .device("engine-1")
            .map(|engine| engine.shutdown)
            .unwrap_or(false);
        if self.snapshot.environment.fire_active && !engine_shutdown {
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

    /// Deterministic (non-backend) drift of every human's stress/attention from
    /// ambient cabin conditions. This is intentionally separate from the
    /// backend-driven decision layer: it models involuntary physiological
    /// response to alarms/smoke, not a deliberate choice.
    fn apply_human_influence(&mut self, events: &mut Vec<EventEnvelope>) {
        let alarm_active = self.snapshot.alarm.active;
        let smoke_density = self.snapshot.environment.smoke_density;
        // Collect (human id, new stress) for humans whose stress changed, so we
        // can emit events after the mutable borrow of `humans` ends; `event`
        // borrows `&mut self`, which would conflict with iterating `humans`.
        let mut changed: Vec<(String, f64)> = Vec::new();
        for human in &mut self.snapshot.humans {
            let old_stress = human.stress;
            if alarm_active || smoke_density > 0.2 {
                human.stress = (human.stress + 0.04).clamp(0.0, 1.0);
                human.attention = (human.attention - 0.02).clamp(0.0, 1.0);
            }
            if (human.stress - old_stress).abs() > f64::EPSILON {
                changed.push((human.id.clone(), human.stress));
            }
        }
        for (human_id, stress) in changed {
            events.push(self.event(
                "StressChanged",
                "human-system",
                Some(&human_id),
                Some(stress),
                "smoke or alarm changed human stress",
            ));
        }
    }

    fn pending_action_write_set(&self) -> std::collections::BTreeSet<&'static str> {
        self.pending_actions
            .iter()
            .flat_map(|action| action.command.write_set().iter().copied())
            .collect()
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
                    if let Some(engine) = self.snapshot.device_mut("engine-1") {
                        engine.shutdown = true;
                        engine.lifecycle = DeviceLifecycle::Recovering;
                    }
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
                Command::ClimateComfortRestore => {
                    self.snapshot.cockpit_systems.climate.comfort_target_c = Some(25.5);
                    self.snapshot.cockpit_systems.climate.cooling_active = true;
                    self.snapshot
                        .cockpit_systems
                        .climate
                        .seat_ventilation_active = true;
                    self.snapshot.environment.temperature_c = 25.5;
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("hvac-1"),
                        None,
                        "climate comfort restoration action applied",
                    ));
                    events.push(self.event(
                        "ThermalComfortRestored",
                        "hvac-1",
                        Some("cabin"),
                        Some(25.5),
                        "HVAC restored the cabin comfort target",
                    ));
                }
                Command::WindshieldDefogActivate => {
                    self.snapshot.cockpit_systems.climate.defog_active = true;
                    self.snapshot.environment.visibility = 0.85;
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("defogger-1"),
                        None,
                        "windshield defog action applied",
                    ));
                    events.push(self.event(
                        "WindshieldVisibilityRestored",
                        "defogger-1",
                        Some("cabin"),
                        Some(0.85),
                        "defogger restored windshield visibility",
                    ));
                }
                Command::FatigueInterventionActivate => {
                    self.snapshot
                        .cockpit_systems
                        .driver_assistance
                        .fatigue_intervention_active = true;
                    if let Some(driver) = self.snapshot.human_mut("driver-1") {
                        driver.attention = 0.72;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("dms-1"),
                        None,
                        "fatigue intervention action applied",
                    ));
                    events.push(self.event(
                        "DriverAttentionRestored",
                        "dms-1",
                        Some("driver-1"),
                        Some(0.72),
                        "fatigue intervention restored driver attention",
                    ));
                }
                Command::ChildProtectionActivate => {
                    self.snapshot
                        .cockpit_systems
                        .occupant_care
                        .child_protection_active = true;
                    self.snapshot
                        .cockpit_systems
                        .occupant_care
                        .emergency_contacted = true;
                    self.snapshot
                        .cockpit_systems
                        .occupant_care
                        .guardian_notified = true;
                    self.snapshot
                        .cockpit_systems
                        .occupant_care
                        .remote_unlock_requested = true;
                    self.snapshot.environment.temperature_c = 29.0;
                    if let Some(child) = self.snapshot.human_mut("child-1") {
                        child.stress = 0.3;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("occupant-radar-1"),
                        None,
                        "child protection action applied",
                    ));
                    events.push(self.event(
                        "ChildProtectionActivated",
                        "occupant-radar-1",
                        Some("cabin"),
                        Some(29.0),
                        "child protection cooled the cabin and contacted emergency support",
                    ));
                }
                Command::MedicalResponseActivate => {
                    self.snapshot
                        .cockpit_systems
                        .occupant_care
                        .medical_response_active = true;
                    self.snapshot
                        .cockpit_systems
                        .occupant_care
                        .emergency_contacted = true;
                    self.snapshot
                        .cockpit_systems
                        .connectivity
                        .emergency_call_active = true;
                    self.snapshot
                        .cockpit_systems
                        .mobility
                        .emergency_route_active = true;
                    if let Some(patient) = self.snapshot.human_mut("patient-1") {
                        patient.stress = 0.35;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("emergency-call-1"),
                        None,
                        "medical response action applied",
                    ));
                    events.push(self.event(
                        "MedicalResponseActivated",
                        "emergency-call-1",
                        Some("patient-1"),
                        Some(0.35),
                        "medical response stabilized the patient and shared location",
                    ));
                }
                Command::PrivacyModeActivate => {
                    self.snapshot.cockpit_systems.experience.privacy_mode_active = true;
                    self.snapshot
                        .cockpit_systems
                        .experience
                        .media_sessions_isolated = true;
                    self.snapshot
                        .cockpit_systems
                        .experience
                        .occupant_profiles_isolated = true;
                    if let Some(driver) = self.snapshot.human_mut("driver-1") {
                        driver.attention = 0.82;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("voice-array-1"),
                        None,
                        "privacy mode action applied",
                    ));
                    events.push(self.event(
                        "PrivacyConflictContained",
                        "voice-array-1",
                        Some("driver-1"),
                        Some(0.82),
                        "voice privacy mode isolated private content and reduced distraction",
                    ));
                }
                Command::ChargingPlanAccept => {
                    self.snapshot
                        .cockpit_systems
                        .experience
                        .charging_plan_accepted = true;
                    self.snapshot.cockpit_systems.mobility.charging_route_active = true;
                    self.snapshot
                        .cockpit_systems
                        .mobility
                        .charger_service_connected = true;
                    if let Some(driver) = self.snapshot.human_mut("driver-1") {
                        driver.stress = 0.35;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("navigation-1"),
                        None,
                        "charging plan acceptance action applied",
                    ));
                    events.push(self.event(
                        "ChargingPlanAccepted",
                        "navigation-1",
                        Some("driver-1"),
                        Some(0.35),
                        "navigation accepted a safe charging route and reduced range anxiety",
                    ));
                }
                Command::AdasTakeoverAcknowledge => {
                    self.snapshot
                        .cockpit_systems
                        .driver_assistance
                        .takeover_acknowledged = true;
                    self.snapshot
                        .cockpit_systems
                        .driver_assistance
                        .takeover_hmi_active = true;
                    if let Some(driver) = self.snapshot.human_mut("driver-1") {
                        driver.attention = 0.92;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("adas-controller-1"),
                        None,
                        "ADAS takeover acknowledgement applied",
                    ));
                    events.push(self.event(
                        "AdasTakeoverCompleted",
                        "adas-controller-1",
                        Some("driver-1"),
                        Some(0.92),
                        "driver acknowledged takeover and restored manual attention",
                    ));
                }
                Command::CyberSafeModeActivate => {
                    self.snapshot.cockpit_systems.cybersecurity.safe_mode_active = true;
                    self.snapshot.cockpit_systems.cybersecurity.network_isolated = true;
                    self.snapshot
                        .cockpit_systems
                        .cybersecurity
                        .identity_verified = true;
                    self.snapshot
                        .cockpit_systems
                        .connectivity
                        .remote_services_isolated = true;
                    self.snapshot
                        .cockpit_systems
                        .connectivity
                        .trusted_local_alert_active = true;
                    if let Some(driver) = self.snapshot.human_mut("driver-1") {
                        driver.attention = 0.88;
                    }
                    events.push(self.event(
                        "ActionApplied",
                        "action-gateway",
                        Some("security-monitor-1"),
                        None,
                        "cybersecurity safe mode action applied",
                    ));
                    events.push(self.event(
                        "CyberIncidentContained",
                        "security-monitor-1",
                        Some("driver-1"),
                        Some(0.88),
                        "security monitor isolated remote control and retained safe functions",
                    ));
                }
            }
        }
    }

    fn apply_pending_human_state_deltas(
        &mut self,
        action_write_set: &std::collections::BTreeSet<&'static str>,
        events: &mut Vec<EventEnvelope>,
    ) {
        let mut deltas = std::mem::take(&mut self.pending_human_state_deltas);
        deltas.sort_by(|left, right| left.human_id.cmp(&right.human_id));
        for delta in deltas {
            let stress_path = format!("{}.stress", delta.human_id);
            let attention_path = format!("{}.attention", delta.human_id);
            if (delta.stress_delta.is_some() && action_write_set.contains(stress_path.as_str()))
                || (delta.attention_delta.is_some()
                    && action_write_set.contains(attention_path.as_str()))
            {
                events.push(self.event_with_error(
                    "HumanStateDeltaRejected",
                    "human-intent",
                    Some(&delta.human_id),
                    Some(ErrorCode::ActionConflict.stable_code().to_string()),
                    "human state delta conflicts with an action effect in this tick",
                ));
                continue;
            }
            if let Some(human) = self.snapshot.human_mut(&delta.human_id) {
                if let Some(value) = delta.stress_delta {
                    human.stress = (human.stress + value).clamp(0.0, 1.0);
                }
                if let Some(value) = delta.attention_delta {
                    human.attention = (human.attention + value).clamp(0.0, 1.0);
                }
            }
            events.push(self.event(
                "HumanStateDeltaApplied",
                "human-intent",
                Some(&delta.human_id),
                None,
                "validated human state delta applied during tick commit",
            ));
        }
    }

    /// Enqueue this tick's world events into every human's physical perception
    /// queue with a deterministic per-human delay, then compact older
    /// delivered memory so the queue stays bounded. Only a fixed allow-list of
    /// event types are treated as human-perceivable (state-machine bookkeeping
    /// events like `StateDiffApplied` are not); the source location used for
    /// delay computation is `"cabin"` for every currently perceivable event
    /// type, since all of them originate from the cabin environment or its
    /// devices.
    fn apply_perception(&mut self, tick: u64, events: &[EventEnvelope]) {
        const PERCEIVABLE_EVENT_TYPES: &[&str] = &[
            "SmokeDetected",
            "SmokeDensityChanged",
            "VisibilityChanged",
            "CabinTemperatureChanged",
            "EngineFire",
            "EngineShutdown",
            "ActionApplied",
            "ThermalComfortRestored",
            "WindshieldVisibilityRestored",
            "DriverAttentionRestored",
            "ChildProtectionActivated",
            "MedicalResponseActivated",
            "PrivacyConflictContained",
            "ChargingPlanAccepted",
            "AdasTakeoverCompleted",
            "CyberIncidentContained",
        ];
        for event in events {
            if !PERCEIVABLE_EVENT_TYPES.contains(&event.event_type.as_str()) {
                continue;
            }
            crate::perception::enqueue_physical_event(
                &mut self.snapshot,
                tick,
                "cabin",
                &event.source,
                &event.event_type,
                &event.payload.message,
            );
        }
        for human in &mut self.snapshot.humans {
            crate::perception::compact_memory(human, tick, 20);
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
        ("cabin", "environment.visibility") | ("engine-1", "engine.health") => {
            (0.0..=1.0).contains(&value)
        }
        ("cabin", "environment.temperatureC") => (-80.0..=100.0).contains(&value),
        ("alarm-1", "alarm.active") => (0.0..=1.0).contains(&value),
        (_, "pilot.stress") | (_, "pilot.attention") => (0.0..=1.0).contains(&value),
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
        ("engine-1", "engine.health") => snapshot.device("engine-1").map(|engine| engine.health),
        ("alarm-1", "alarm.active") => Some(if snapshot.alarm.active { 1.0 } else { 0.0 }),
        (human_id, "pilot.stress") => snapshot.human(human_id).map(|human| human.stress),
        (human_id, "pilot.attention") => snapshot.human(human_id).map(|human| human.attention),
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
        ("engine-1", "engine.health") => {
            if let Some(engine) = snapshot.device_mut("engine-1") {
                engine.health = value;
            }
        }
        ("alarm-1", "alarm.active") => snapshot.alarm.active = value > 0.5,
        (human_id, "pilot.stress") => {
            if let Some(human) = snapshot.human_mut(human_id) {
                human.stress = value;
            }
        }
        (human_id, "pilot.attention") => {
            if let Some(human) = snapshot.human_mut(human_id) {
                human.attention = value;
            }
        }
        _ => unreachable!("component path is validated before write"),
    }
}
