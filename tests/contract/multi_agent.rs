use cockpit_agent_runtime::{AgentActionBatch, MultiAgentCoordinator};
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::{ActionRequest, ActionStatus, Command, Simulation};

fn action(agent_id: &str, request_id: &str, version: u64) -> ActionRequest {
    ActionRequest {
        request_id: request_id.to_string(),
        agent_id: agent_id.to_string(),
        target: "engine-1".to_string(),
        command: Command::EngineShutdown,
        expected_state_version: version,
        expires_at_tick: 3,
        correlation_id: format!("{request_id}-correlation"),
    }
}

#[test]
fn multi_agent_arbitration_is_priority_then_stable_and_conflict_safe() {
    let mut scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    scenario.agents.push(cockpit_simulation_core::AgentGrant {
        agent_id: "copilot-agent".to_string(),
        capabilities: vec!["engine.shutdown".to_string()],
    });
    let mut simulation = Simulation::new("multi-agent-run", scenario);
    simulation.start().expect("run starts");
    let version = simulation.snapshot.version;
    let results = MultiAgentCoordinator.submit_batches(
        &mut simulation,
        vec![
            AgentActionBatch {
                agent_id: "copilot-agent".to_string(),
                priority: 10,
                actions: vec![action("copilot-agent", "copilot-shutdown", version)],
            },
            AgentActionBatch {
                agent_id: "cockpit-agent".to_string(),
                priority: 1,
                actions: vec![action("cockpit-agent", "pilot-shutdown", version)],
            },
        ],
    );

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].status, ActionStatus::Applied);
    assert_eq!(results[1].status, ActionStatus::Rejected);
    assert_eq!(
        results[1].error_code,
        Some(cockpit_simulation_core::ErrorCode::ActionConflict)
    );
    simulation.step_without_agent().expect("tick commits");
    assert!(simulation.snapshot.engine.shutdown);
}
