use cockpit_agent_runtime::{acp_adapter::IotaCoreAcpAdapter, iota_core_adapter::IotaCoreAdapter};
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::Simulation;

#[test]
fn acp_prompt_contains_only_perceived_observation_data() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let simulation = Simulation::new("acp-contract-run", scenario);
    let skill = IotaCoreAdapter::new(env!("CARGO_MANIFEST_DIR"))
        .load_cockpit_skill()
        .expect("skill loads");
    let prompt = IotaCoreAcpAdapter::build_prompt(&simulation.observation(), &skill);

    assert!(prompt.contains("Never request or infer Ground Truth"));
    assert!(prompt.contains("visibleEntities"));
    assert!(prompt.contains("confidence"));
    assert!(!prompt.contains("smokeDensity"));
    assert!(!prompt.contains("fireActive"));
    assert!(!prompt.contains("Ground Truth JSON"));
}
