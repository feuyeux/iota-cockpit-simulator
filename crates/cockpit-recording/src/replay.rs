use cockpit_simulation_core::{Simulation, SimulationScenario, error::SimulationResult};

use crate::Recording;

pub fn replay_recording(
    run_id: impl Into<String>,
    scenario: SimulationScenario,
    source: &Recording,
) -> SimulationResult<Recording> {
    if source.scenario_hash != scenario.scenario_hash {
        return Err(cockpit_simulation_core::SimulationError::InvalidScenario(
            "recording scenario hash does not match scenario".to_string(),
        ));
    }

    let run_id = run_id.into();
    let mut simulation = Simulation::new(run_id.clone(), scenario.clone());
    simulation.start()?;
    let mut replay = Recording::new(run_id, &scenario);
    let actions_by_tick = source.recorded_actions_by_tick();

    for tick in &source.ticks {
        let actions = actions_by_tick.get(&tick.tick).cloned().unwrap_or_default();
        let step = simulation.step_with_recorded_actions(actions)?;
        replay.push(step);
    }
    Ok(replay)
}
