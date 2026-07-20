use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    action::{ActionRequest, ActionResult, ActionStatus, AgentGrant, ErrorCode, ScriptedAgent},
    capability::CapabilityCatalog,
    clock::{ClockConfig, RunStatus},
    digital_twin::DigitalTwinParameters,
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
    /// Runtime-owned physics profile. It is not parsed from public scenario
    /// YAML, preventing scenarios from silently redefining calibration truth.
    #[serde(default)]
    pub physics: DigitalTwinParameters,
    pub faults: Vec<Fault>,
    pub agent: AgentGrant,
    #[serde(default)]
    pub agents: Vec<AgentGrant>,
    /// Public, non-scoring objectives visible to operators and agents. They
    /// describe desired world outcomes without revealing evaluator thresholds,
    /// action mappings, or release gates.
    #[serde(default)]
    pub public_goals: Vec<String>,
    /// Runtime horizon only; independent private rubrics own all deadlines.
    #[serde(default = "default_max_ticks")]
    pub max_ticks: u64,
    /// Scheduled, versioned influence rules applied during tick commit. Empty by
    /// default, so scenarios without influences keep identical tick behavior.
    #[serde(default)]
    pub influences: Vec<InfluenceRule>,
    /// Conflict policy used when multiple influence rules target the same
    /// component in one tick.
    #[serde(default = "default_conflict_policy")]
    pub conflict_policy: ConflictPolicy,
}

impl SimulationScenario {
    /// The scenario's primary human (first entry in `humans`), i.e. the human
    /// operated/observed by the primary `agent` grant. `load_scenario`
    /// guarantees at least one human at parse time, but this remains
    /// fallible so callers cannot assume panic-free indexing if that
    /// invariant is ever relaxed.
    pub fn primary_human(&self) -> Option<&HumanState> {
        self.humans.first()
    }

    /// The scenario's primary device (first entry in `devices`). Unlike
    /// `humans`, scenarios are not required to declare any `device` entity,
    /// so this can legitimately be `None`.
    pub fn primary_device(&self) -> Option<&DeviceState> {
        self.devices.first()
    }
}

fn default_conflict_policy() -> ConflictPolicy {
    ConflictPolicy::RejectConflicting
}

fn default_max_ticks() -> u64 {
    80
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
    capabilities: CapabilityCatalog,
    sequence: u64,
    pending_actions: Vec<ActionRequest>,
    pending_human_state_deltas: Vec<HumanStateDelta>,
    pending_lifecycle_events: Vec<EventEnvelope>,
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
            capabilities: CapabilityCatalog::load_default(),
            sequence: 0,
            pending_actions: Vec::new(),
            pending_human_state_deltas: Vec::new(),
            pending_lifecycle_events: Vec::new(),
            latest_results: Vec::new(),
        }
    }

    /// Reconstruct a transaction-local simulation view from a trusted snapshot.
    ///
    /// This is used by the stdio MCP bridge to validate native tool calls in an
    /// isolated child process. Pending actions and human deltas intentionally
    /// start empty; the parent driver replays every accepted native call into
    /// its own cloned transaction before committing the tick.
    pub fn from_tool_snapshot(
        scenario: SimulationScenario,
        status: RunStatus,
        snapshot: WorldSnapshot,
    ) -> Self {
        Self {
            scenario,
            status,
            snapshot,
            capabilities: CapabilityCatalog::load_default(),
            sequence: 0,
            pending_actions: Vec::new(),
            pending_human_state_deltas: Vec::new(),
            pending_lifecycle_events: Vec::new(),
            latest_results: Vec::new(),
        }
    }

    /// The capability catalog this simulation resolves actions against.
    pub fn capabilities(&self) -> &CapabilityCatalog {
        &self.capabilities
    }

    pub fn run_id(&self) -> &str {
        &self.snapshot.run_id
    }

    pub fn spawn_entity(&mut self, entity: crate::world::DynamicEntity) -> SimulationResult<()> {
        let entity_id = entity.id().to_string();
        let entity_kind = match &entity {
            crate::world::DynamicEntity::Human(_) => "human",
            crate::world::DynamicEntity::Device(_) => "device",
        };
        self.snapshot
            .spawn_entity(entity)
            .map_err(SimulationError::InvalidScenario)?;
        self.snapshot.version = self.snapshot.version.saturating_add(1);
        let event = self.event(
            "EntitySpawned",
            "world-kernel",
            Some(&entity_id),
            None,
            &format!("dynamic {entity_kind} entity spawned"),
        );
        self.pending_lifecycle_events.push(event);
        Ok(())
    }

    pub fn remove_entity(
        &mut self,
        entity_id: &str,
    ) -> SimulationResult<crate::world::DynamicEntity> {
        let entity = self.snapshot.remove_entity(entity_id).ok_or_else(|| {
            SimulationError::InvalidScenario(format!("entity '{entity_id}' was not found"))
        })?;
        let entity_kind = match &entity {
            crate::world::DynamicEntity::Human(_) => "human",
            crate::world::DynamicEntity::Device(_) => "device",
        };
        self.snapshot.version = self.snapshot.version.saturating_add(1);
        let event = self.event(
            "EntityRemoved",
            "world-kernel",
            Some(entity_id),
            None,
            &format!("dynamic {entity_kind} entity removed"),
        );
        self.pending_lifecycle_events.push(event);
        Ok(entity)
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
                .allows(&request.agent_id, &request.capability_id)
        } else {
            self.scenario
                .agents
                .iter()
                .any(|agent| agent.allows(&request.agent_id, &request.capability_id))
        };
        let capability = self.capabilities.get(&request.capability_id);
        let error_code = if !authorized || capability.is_none() {
            Some(ErrorCode::CapabilityDenied)
        } else if request.expires_at_tick < self.snapshot.tick {
            Some(ErrorCode::ActionExpired)
        } else if request.expected_state_version != self.snapshot.version {
            Some(ErrorCode::VersionMismatch)
        } else if self.pending_actions.iter().any(|pending| {
            let Some(pending_capability) = self.capabilities.get(&pending.capability_id) else {
                return false;
            };
            let write_set = capability
                .map(|definition| definition.write_set.as_slice())
                .unwrap_or_default();
            pending_capability
                .write_set
                .iter()
                .any(|path| write_set.contains(path))
        }) {
            Some(ErrorCode::ActionConflict)
        } else if capability.is_some_and(|definition| request.target != definition.target_id) {
            Some(ErrorCode::UnknownTarget)
        } else {
            crate::effects::validate_action(&self.capabilities, &self.snapshot, request).err()
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
        let mut events = self.pending_lifecycle_events.clone();

        self.apply_digital_twin(&mut events)?;
        for fault in self.scenario.faults.clone() {
            if fault.at_tick == tick {
                self.apply_fault(&fault, &mut events);
            }
        }
        self.apply_influences(tick, &mut events);
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
        self.pending_lifecycle_events.clear();

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
            self.snapshot.environment.smoke_density =
                self.snapshot.environment.smoke_density.max(0.18);
            self.snapshot.environment.visibility = self.snapshot.environment.visibility.min(0.4);
            let smoke_mg_m3 = self.snapshot.environment.smoke_density
                / self.scenario.physics.smoke_mass_extinction_m2_mg;
            for zone in self.snapshot.environment.zones.values_mut() {
                zone.smoke_mg_m3 = zone.smoke_mg_m3.max(smoke_mg_m3);
            }
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
            events.push(self.event(
                "SmokeDetected",
                "sensor-system",
                Some("cabin"),
                Some(self.snapshot.environment.visibility),
                "fault introduced smoke at the detection threshold",
            ));
        }
    }

    /// Advance calibrated cabin thermodynamics, humidity, pressure, smoke/CO₂/CO
    /// balances, and occupant physiology as one coupled transaction.
    fn apply_digital_twin(&mut self, events: &mut Vec<EventEnvelope>) -> SimulationResult<()> {
        let elapsed_s = self.scenario.clock.tick_ms as f64 / 1_000.0;
        let step =
            crate::digital_twin::advance(&mut self.snapshot, &self.scenario.physics, elapsed_s)
                .map_err(SimulationError::InvalidScenario)?;
        if step.energy_residual_j.abs() > 0.01 || step.contaminant_residual_mg.abs() > 0.001 {
            return Err(SimulationError::InvalidScenario(format!(
                "digital-twin conservation residual exceeded tolerance: energy={}J contaminant={}mg",
                step.energy_residual_j, step.contaminant_residual_mg
            )));
        }
        if (step.temperature_c - step.previous_temperature_c).abs() > f64::EPSILON {
            events.push(self.event(
                "CabinTemperatureChanged",
                "calibrated-digital-twin",
                Some("cabin"),
                Some(step.temperature_c),
                "calibrated multi-zone energy balance changed cabin temperature",
            ));
        }
        if (step.visibility - step.previous_visibility).abs() > f64::EPSILON {
            events.push(self.event(
                "VisibilityChanged",
                "calibrated-digital-twin",
                Some("cabin"),
                Some(step.visibility),
                "Beer-Lambert smoke extinction changed cockpit visibility",
            ));
        }
        if (step.smoke_density - step.previous_smoke_density).abs() > f64::EPSILON {
            events.push(self.event(
                "SmokeDensityChanged",
                "calibrated-digital-twin",
                Some("cabin"),
                Some(step.smoke_density),
                "conserved smoke mass changed optical extinction",
            ));
        }
        if self.snapshot.environment.fire_active && step.smoke_density >= 0.18 {
            events.push(self.event(
                "SmokeDetected",
                "sensor-system",
                Some("cabin"),
                Some(step.visibility),
                "mass-balance smoke concentration reached the detection threshold",
            ));
        }
        events.push(self.event(
            "CabinPressureUpdated",
            "calibrated-digital-twin",
            Some("cabin"),
            Some(step.pressure_pa),
            "barometric leakage and HVAC pressure balance advanced",
        ));
        events.push(self.event(
            "CabinAirQualityUpdated",
            "calibrated-digital-twin",
            Some("cabin"),
            Some(step.carbon_dioxide_ppm),
            "occupant generation and ventilation mass balance advanced",
        ));
        for physiology in step.physiology {
            events.push(self.event(
                "HumanPhysiologyUpdated",
                "two-node-physiology",
                Some(&physiology.human_id),
                Some(physiology.core_temperature_c),
                "two-node thermoregulation and inhalation exposure advanced",
            ));
            if physiology.carboxyhemoglobin_pct >= 2.0 {
                events.push(self.event(
                    "CarbonMonoxideExposure",
                    "two-node-physiology",
                    Some(&physiology.human_id),
                    Some(physiology.carboxyhemoglobin_pct),
                    "carboxyhemoglobin exceeded the physiological evidence threshold",
                ));
            }
        }
        Ok(())
    }

    fn pending_action_write_set(&self) -> std::collections::BTreeSet<String> {
        self.pending_actions
            .iter()
            .filter_map(|action| self.capabilities.get(&action.capability_id))
            .flat_map(|capability| capability.write_set.iter().cloned())
            .collect()
    }

    fn apply_pending_actions(&mut self, events: &mut Vec<EventEnvelope>) {
        let mut actions = std::mem::take(&mut self.pending_actions);
        actions.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then(left.capability_id.cmp(&right.capability_id))
                .then(left.request_id.cmp(&right.request_id))
        });

        for action in actions {
            let plan = crate::effects::resolve_action(&self.capabilities, &self.snapshot, &action)
                .expect("validated effect plan must resolve against the same transaction snapshot");
            plan.apply(&mut self.snapshot)
                .expect("validated effect plan must apply to the same transaction snapshot");
            for effect_event in plan.events {
                events.push(self.event(
                    &effect_event.event_type,
                    &effect_event.source,
                    effect_event.target.as_deref(),
                    effect_event.value,
                    &effect_event.message,
                ));
            }
        }
    }

    fn apply_pending_human_state_deltas(
        &mut self,
        action_write_set: &std::collections::BTreeSet<String>,
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
