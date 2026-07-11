use cockpit_agent_runtime::{
    LocalMcpServer, TOOL_GET_OBSERVATION, TOOL_REQUEST_ACTION, ToolRequest,
    iota_core_adapter::IotaCoreAdapter,
};
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::Simulation;
use serde_json::json;

fn request(
    run_id: &str,
    agent_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> ToolRequest {
    ToolRequest {
        call_id: format!("call-{tool_name}"),
        run_id: run_id.to_string(),
        agent_id: agent_id.to_string(),
        tick: 0,
        tool_name: tool_name.to_string(),
        arguments,
        correlation_id: "contract-correlation".to_string(),
    }
}

#[test]
fn exposes_phase_one_tools_and_keeps_ground_truth_out_of_observation() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("contract-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();

    let definitions = LocalMcpServer::tool_definitions();
    let names: Vec<_> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert_eq!(names.len(), 6);
    assert!(names.contains(&"simulation.get_observation"));
    assert!(names.contains(&"simulation.list_visible_entities"));
    assert!(names.contains(&"simulation.inspect_sensor_quality"));
    assert!(names.contains(&"simulation.request_action"));
    assert!(names.contains(&"simulation.get_action_result"));
    assert!(names.contains(&"simulation.get_run_status"));
    assert!(
        definitions
            .iter()
            .any(|definition| definition.name == TOOL_REQUEST_ACTION && definition.side_effect)
    );
    assert!(
        definitions
            .iter()
            .any(|definition| definition.name == TOOL_GET_OBSERVATION && !definition.side_effect)
    );

    let (response, trace) = server.call(
        &mut simulation,
        request(
            "contract-run",
            "cockpit-agent",
            TOOL_GET_OBSERVATION,
            json!({}),
        ),
    );
    assert!(response.error.is_none(), "{response:?}");
    assert!(response.result.get("smokeDensity").is_none());
    assert!(response.result.get("environment").is_none());
    assert!(!trace.result.to_string().contains("smokeDensity"));
}

#[test]
fn rejects_unknown_agent_before_action_gateway() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("contract-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    let (response, trace) = server.call(
        &mut simulation,
        request(
            "contract-run",
            "intruder",
            TOOL_REQUEST_ACTION,
            json!({
                "target": "engine-1",
                "command": "engineShutdown",
                "expectedStateVersion": 0,
                "expiresAtTick": 3
            }),
        ),
    );

    assert_eq!(
        response.error.as_ref().map(|error| error.code.as_str()),
        Some("AGENT_IDENTITY_DENIED")
    );
    assert!(!trace.allowed);
    assert!(!simulation.snapshot.engine.shutdown);
}

#[test]
fn iota_core_adapter_loads_cockpit_skill_from_public_registry() {
    let skill = IotaCoreAdapter::new(env!("CARGO_MANIFEST_DIR"))
        .load_cockpit_skill()
        .expect("skill is registered");
    assert_eq!(skill.name, "cockpit-simulation");
    assert_eq!(skill.version, "1");
    assert!(skill.body.contains("Never request or infer Ground Truth"));
    assert!(
        skill
            .tools
            .contains(&"simulation.request_action".to_string())
    );
}
