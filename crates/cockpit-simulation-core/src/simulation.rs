use serde::{Deserialize, Serialize};

use crate::{
    action::{
        ActionRequest, ActionResult, ActionStatus, AgentGrant, Command, ErrorCode, ScriptedAgent,
    },
    clock::{ClockConfig, RunStatus},
    error::{SimulationError, SimulationResult},
    event::{EventEnvelope, EventPayload, ToolCallTrace},
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
        self.commit_step(observation)
    }

    pub fn step_with_recorded_actions(
        &mut self,
        actions: Vec<ActionRequest>,
    ) -> SimulationResult<StepRecord> {
        let observation = Observation::from_snapshot(
            self.run_id(),
            &self.scenario.agent.agent_id,
            &self.snapshot,
        );
        for action in actions {
            self.submit_action(action);
        }
        self.commit_step(observation)
    }

    pub fn step_without_agent(&mut self) -> SimulationResult<StepRecord> {
        let observation = Observation::from_snapshot(
            self.run_id(),
            &self.scenario.agent.agent_id,
            &self.snapshot,
        );
        self.commit_step(observation)
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

    fn commit_step(&mut self, mut observation: Observation) -> SimulationResult<StepRecord> {
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

        self.apply_environment(&mut events);
        self.apply_human_influence(&mut events);
        self.apply_pending_actions(&mut events);

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
        })
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
