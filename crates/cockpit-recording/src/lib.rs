use std::collections::BTreeMap;

use cockpit_agent::{LocalMcpServer, RuleAgent};
use cockpit_world::{
    ScriptedAgent,
    action::{ActionRequest, ActionStatus},
    clock::ClockConfig,
    error::SimulationResult,
    simulation::{Simulation, SimulationScenario, StateDiff, StepRecord},
};
use serde::{Deserialize, Serialize};

pub mod diff;
pub mod queue;
pub mod replay;
pub mod store;

/// Current recording schema version understood by this build. Version 2 adds
/// an optional durable world-plus-agent checkpoint for live restart recovery.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;
/// Current runtime contract version. Version 7 introduces deterministic
/// event-driven human scheduling, so older recordings cannot be replayed
/// against a different backend-consumption schedule.
pub const CURRENT_RUNTIME_CONTRACT_VERSION: u32 = 7;
/// Current world-model version. Version 8 adds humidity-limited evaporative
/// heat loss to two-node occupant thermoregulation; replay rejects prior
/// physiology behavior rather than claiming deterministic equivalence.
pub const CURRENT_WORLD_MODEL_VERSION: u32 = 8;

pub use diff::{RecordingDiff, RecordingMetrics, TickDiff, diff_recordings};
pub use queue::{
    AsyncRecordingSink, RecordingQueue, RecordingQueueHealth, RecordingQueueOutcome,
    RecordingQueuePolicy,
};
pub use replay::replay_recording;
pub use store::{PayloadStore, RecordingStore, RecordingStoreError, serialize_redacted_recording};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub schema_version: u32,
    pub runtime_contract_version: u32,
    pub world_model_version: u32,
    pub application_commit: String,
    pub plugin_hashes: Vec<String>,
    pub run_id: String,
    pub scenario_id: String,
    pub scenario_hash: String,
    pub seed: u64,
    pub clock: ClockConfig,
    pub ticks: Vec<StepRecord>,
    /// Per-tick human decisions for a live run, in driver order. Free-form
    /// narrative and utterance text is redacted; typed actions and state deltas
    /// remain available for deterministic replay without another model call.
    #[serde(default)]
    pub human_turns: Vec<Vec<cockpit_agent::HumanTurnEvidence>>,
    #[serde(default)]
    pub provenance: RunProvenance,
    /// Latest restartable world-plus-agent control-plane checkpoint for live
    /// runs. Evaluators may inspect it but never mutate it.
    #[serde(default)]
    pub open_world_checkpoint: Option<cockpit_agent::OpenWorldCheckpoint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProvenance {
    pub suite_id: Option<String>,
    pub suite_version: Option<String>,
    pub split: Option<String>,
    pub backend: Option<String>,
    pub variant_hash: Option<String>,
    pub prompt_template_hash: Option<String>,
    pub skill_hash: Option<String>,
}

impl Recording {
    pub fn new(run_id: impl Into<String>, scenario: &SimulationScenario) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            runtime_contract_version: CURRENT_RUNTIME_CONTRACT_VERSION,
            world_model_version: CURRENT_WORLD_MODEL_VERSION,
            application_commit: option_env!("COCKPIT_APPLICATION_COMMIT")
                .unwrap_or("unknown")
                .to_string(),
            plugin_hashes: Vec::new(),
            run_id: run_id.into(),
            scenario_id: scenario.id.clone(),
            scenario_hash: scenario.scenario_hash.clone(),
            seed: scenario.seed,
            clock: scenario.clock,
            ticks: Vec::new(),
            human_turns: Vec::new(),
            provenance: RunProvenance::default(),
            open_world_checkpoint: None,
        }
    }

    pub fn push(&mut self, step: StepRecord) {
        self.ticks.push(step);
    }

    /// Record one tick's backend-authored human decisions (live runs only).
    pub fn push_human_turns(&mut self, turns: Vec<cockpit_agent::HumanTurnEvidence>) {
        self.human_turns.push(turns);
    }

    pub fn final_snapshot_hash(&self) -> Option<&str> {
        self.ticks.last().map(|tick| tick.snapshot_hash.as_str())
    }

    pub fn recorded_actions_by_tick(&self) -> BTreeMap<u64, Vec<ActionRequest>> {
        let mut actions = BTreeMap::new();
        for tick in &self.ticks {
            for result in &tick.action_results {
                if result.status == ActionStatus::Applied {
                    actions
                        .entry(result.tick)
                        .or_insert_with(Vec::new)
                        .push(result.request.clone());
                }
            }
        }
        actions
    }

    pub fn recorded_state_diffs_by_tick(&self) -> BTreeMap<u64, Vec<StateDiff>> {
        self.ticks
            .iter()
            .map(|tick| (tick.tick, tick.state_diffs.clone()))
            .collect()
    }
}

pub fn run_scripted_recording(
    run_id: impl Into<String>,
    scenario: SimulationScenario,
    ticks: u64,
) -> SimulationResult<Recording> {
    let run_id = run_id.into();
    let mut simulation = Simulation::new(run_id.clone(), scenario.clone());
    simulation.start()?;
    let mut recording = Recording::new(run_id, &scenario);
    let mut agent = ScriptedAgent::default();
    for _ in 0..ticks {
        let step = simulation.step_with_scripted_agent(&mut agent)?;
        recording.push(step);
    }
    Ok(recording)
}

pub fn run_rule_agent_recording(
    run_id: impl Into<String>,
    scenario: SimulationScenario,
    ticks: u64,
) -> SimulationResult<Recording> {
    let run_id = run_id.into();
    let mut simulation = Simulation::new(run_id.clone(), scenario.clone());
    simulation.start()?;
    let mut recording = Recording::new(run_id, &scenario);
    let mut server = LocalMcpServer::default();
    let mut agent = RuleAgent::default();
    for _ in 0..ticks {
        recording.push(agent.step(&mut simulation, &mut server)?);
    }
    Ok(recording)
}
