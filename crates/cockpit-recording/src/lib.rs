use std::collections::BTreeMap;

use cockpit_agent_runtime::{LocalMcpServer, RuleAgent};
use cockpit_simulation_core::{
    ScriptedAgent,
    action::{ActionRequest, ActionStatus},
    clock::ClockConfig,
    error::SimulationResult,
    simulation::{Simulation, SimulationScenario, StateDiff, StepRecord},
};
use serde::{Deserialize, Serialize};

pub mod diff;
pub mod migrate;
pub mod queue;
pub mod replay;
pub mod store;

pub use diff::{RecordingDiff, RecordingMetrics, TickDiff, diff_recordings};
pub use migrate::{
    CURRENT_RUNTIME_CONTRACT_VERSION, CURRENT_SCHEMA_VERSION, CURRENT_WORLD_MODEL_VERSION,
    MigrationError, MigrationReport, migrate_recording_bytes, migrate_recording_value,
};
pub use queue::{
    AsyncRecordingSink, RecordingQueue, RecordingQueueHealth, RecordingQueueOutcome,
    RecordingQueuePolicy,
};
pub use replay::replay_recording;
pub use store::{PayloadStore, RecordingStore, RecordingStoreError};

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
}

impl Recording {
    pub fn new(run_id: impl Into<String>, scenario: &SimulationScenario) -> Self {
        Self {
            schema_version: migrate::CURRENT_SCHEMA_VERSION,
            runtime_contract_version: migrate::CURRENT_RUNTIME_CONTRACT_VERSION,
            world_model_version: migrate::CURRENT_WORLD_MODEL_VERSION,
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
        }
    }

    pub fn push(&mut self, step: StepRecord) {
        self.ticks.push(step);
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
