use std::collections::BTreeMap;

use cockpit_agent_runtime::{LocalMcpServer, RuleAgent};
use cockpit_simulation_core::{
    ScriptedAgent,
    action::{ActionRequest, ActionStatus},
    error::SimulationResult,
    simulation::{Simulation, SimulationScenario, StepRecord},
};
use serde::{Deserialize, Serialize};

pub mod replay;
pub mod store;

pub use replay::replay_recording;
pub use store::{RecordingStore, RecordingStoreError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub schema_version: u32,
    pub run_id: String,
    pub scenario_id: String,
    pub scenario_hash: String,
    pub seed: u64,
    pub ticks: Vec<StepRecord>,
}

impl Recording {
    pub fn new(run_id: impl Into<String>, scenario: &SimulationScenario) -> Self {
        Self {
            schema_version: 1,
            run_id: run_id.into(),
            scenario_id: scenario.id.clone(),
            scenario_hash: scenario.scenario_hash.clone(),
            seed: scenario.seed,
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
