use cockpit_agent::{
    LocalMcpServer, OpenWorldControlRequest, TOOL_ADD_GOAL, TOOL_GET_ACTION_RESULT,
    TOOL_GET_OBSERVATION, TOOL_GET_TURN_CONTEXT, TOOL_LIST_VISIBLE_ENTITIES, TOOL_REQUEST_ACTION,
    TOOL_SUBMIT_DECISION, TOOL_WAIT_UNTIL, ToolRequest, iota_core_adapter::IotaCoreAdapter,
};
use cockpit_scenario::load_scenario;
use cockpit_world::Simulation;
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
        human_id: None,
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
    assert_eq!(names.len(), 10);
    assert!(names.contains(&"simulation.get_turn_context"));
    assert!(names.contains(&"simulation.get_observation"));
    assert!(names.contains(&"simulation.list_visible_entities"));
    assert!(names.contains(&"simulation.inspect_sensor_quality"));
    assert!(names.contains(&"simulation.request_action"));
    assert!(names.contains(&"simulation.submit_decision"));
    assert!(names.contains(&"simulation.get_action_result"));
    assert!(names.contains(&"simulation.get_run_status"));
    assert!(names.contains(&"simulation.add_goal"));
    assert!(names.contains(&"simulation.wait_until"));
    assert!(definitions.iter().any(|definition| {
        matches!(definition.name.as_str(), TOOL_ADD_GOAL | TOOL_WAIT_UNTIL)
            && definition.side_effect
    }));
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
    assert!(
        definitions
            .iter()
            .any(|definition| definition.name == TOOL_SUBMIT_DECISION && !definition.side_effect)
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

    let mut turn_context_request = request(
        "contract-run",
        "cockpit-agent",
        TOOL_GET_TURN_CONTEXT,
        json!({}),
    );
    turn_context_request.human_id = Some("pilot-1".to_string());
    let (turn_context, trace) = server.call(&mut simulation, turn_context_request);
    assert!(turn_context.error.is_none(), "{turn_context:?}");
    assert!(turn_context.result.get("observation").is_some());
    assert!(turn_context.result.get("sensorQuality").is_some());
    assert!(turn_context.result.get("stateVersion").is_some());
    assert!(!turn_context.result.to_string().contains("smokeDensity"));
    assert!(!trace.side_effect);
}

#[test]
fn human_scoped_action_cannot_borrow_the_primary_agent_grant() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("human-scope-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    let mut tool_request = request(
        "human-scope-run",
        "cockpit-agent",
        TOOL_REQUEST_ACTION,
        json!({
            "target": "engine-1",
            "command": "engineShutdown",
            "expectedStateVersion": 0,
            "expiresAtTick": 3
        }),
    );
    tool_request.human_id = Some("rear-passenger-1".to_string());

    let (response, trace) = server.call(&mut simulation, tool_request);

    assert_eq!(
        response.error.as_ref().map(|error| error.code.as_str()),
        Some("HUMAN_CAPABILITY_DENIED")
    );
    assert!(!trace.allowed);
    assert!(!simulation.snapshot.device("engine-1").unwrap().shutdown);
}

#[test]
fn action_results_are_owned_by_the_authenticated_human() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("result-owner-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    let mut action = request(
        "result-owner-run",
        "cockpit-agent",
        TOOL_REQUEST_ACTION,
        json!({
            "target": "engine-1",
            "command": "engineShutdown",
            "expectedStateVersion": 0,
            "expiresAtTick": 3
        }),
    );
    action.human_id = Some("pilot-1".to_string());
    let (action_response, _) = server.call(&mut simulation, action);
    assert!(action_response.error.is_none(), "{action_response:?}");

    let mut foreign_read = request(
        "result-owner-run",
        "cockpit-agent",
        TOOL_GET_ACTION_RESULT,
        json!({ "requestId": "call-simulation.request_action" }),
    );
    foreign_read.human_id = Some("rear-passenger-1".to_string());
    let (response, trace) = server.call(&mut simulation, foreign_read);

    assert_eq!(
        response.error.as_ref().map(|error| error.code.as_str()),
        Some("ACTION_RESULT_IDENTITY_DENIED")
    );
    assert!(
        trace.allowed,
        "identity denial is a tool-level ownership result"
    );
}

#[test]
fn visible_entities_are_cursor_paginated() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("pagination-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    let mut paged = request(
        "pagination-run",
        "cockpit-agent",
        TOOL_LIST_VISIBLE_ENTITIES,
        json!({ "cursor": 0, "limit": 1 }),
    );
    paged.human_id = Some("pilot-1".to_string());

    let (response, _) = server.call(&mut simulation, paged);

    assert!(response.error.is_none(), "{response:?}");
    assert_eq!(
        response.result["entities"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(response.result["page"]["cursor"], 0);
    assert_eq!(response.result["page"]["limit"], 1);
    assert!(response.result["page"].get("total").is_some());
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
    assert!(!simulation.snapshot.device("engine-1").unwrap().shutdown);
}

#[test]
fn rejected_action_is_published_as_a_stable_event() {
    let mut scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    scenario.agent.capabilities = vec!["engine.shutdown".to_string()];
    scenario.agents.clear();
    let mut simulation = Simulation::new("rejection-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    let (response, _) = server.call(
        &mut simulation,
        request(
            "rejection-run",
            "cockpit-agent",
            TOOL_REQUEST_ACTION,
            json!({
                "target": "alarm-1",
                "command": "alarmActivate",
                "expectedStateVersion": 0,
                "expiresAtTick": 3
            }),
        ),
    );
    assert_eq!(
        response
            .result
            .get("errorCode")
            .and_then(|value| value.as_str()),
        Some("CAPABILITY_DENIED")
    );
    let step = simulation.step_without_agent().expect("tick commits");
    let event = step
        .events
        .iter()
        .find(|event| event.event_type == "ActionRejected")
        .expect("rejection event");
    assert_eq!(
        event.payload.error_code.as_deref(),
        Some("CAPABILITY_DENIED")
    );
}

#[test]
fn mutation_requires_approval_when_the_runtime_policy_enables_it() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("approval-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    server.set_approval_required(true);
    let (response, _) = server.call(
        &mut simulation,
        request(
            "approval-run",
            "cockpit-agent",
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
        response
            .result
            .get("status")
            .and_then(|value| value.as_str()),
        Some("pendingApproval")
    );
    assert!(!simulation.snapshot.device("engine-1").unwrap().shutdown);

    let result = server
        .approve_action(&mut simulation, "call-simulation.request_action")
        .expect("approval applies");
    assert_eq!(result.status, cockpit_world::ActionStatus::Applied);
    simulation.step_without_agent().expect("tick commits");
    assert!(simulation.snapshot.device("engine-1").unwrap().shutdown);
}

#[test]
fn domain_action_requires_approval_before_world_state_changes() {
    let scenario =
        load_scenario("scenarios/heatwave-thermal-comfort.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("domain-approval-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    server.set_approval_required(true);

    let (response, _) = server.call(
        &mut simulation,
        request(
            "domain-approval-run",
            "cockpit-agent",
            TOOL_REQUEST_ACTION,
            json!({
                "target": "hvac-1",
                "command": "climateComfortRestore",
                "expectedStateVersion": 0,
                "expiresAtTick": 3
            }),
        ),
    );

    assert_eq!(
        response
            .result
            .get("status")
            .and_then(|value| value.as_str()),
        Some("pendingApproval")
    );
    assert!(!simulation.snapshot.cockpit_systems.climate.cooling_active);
    assert_eq!(simulation.snapshot.environment.temperature_c, 43.0);

    let result = server
        .approve_action(&mut simulation, "call-simulation.request_action")
        .expect("approval applies");
    assert_eq!(result.status, cockpit_world::ActionStatus::Applied);
    simulation
        .step_without_agent()
        .expect("approved action commits");
    assert!(simulation.snapshot.cockpit_systems.climate.cooling_active);
    assert_eq!(simulation.snapshot.environment.temperature_c, 25.5);
}

#[test]
fn iota_core_adapter_loads_cockpit_skill_from_public_registry() {
    let skill = IotaCoreAdapter::new(env!("CARGO_MANIFEST_DIR"))
        .load_cockpit_skill()
        .expect("skill is registered");
    assert_eq!(skill.name, "cockpit-world");
    assert_eq!(skill.version, "6");
    assert!(skill.body.contains("Never request or infer Ground Truth"));
    assert!(
        skill
            .body
            .contains("Only `simulation.request_action` may mutate the physical world")
    );
    assert!(
        skill
            .tools
            .contains(&"simulation.request_action".to_string())
    );
    assert!(skill.tools.contains(&TOOL_GET_TURN_CONTEXT.to_string()));
}

#[test]
fn tool_trace_redacts_nested_secret_arguments_before_recording() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("redaction-run", scenario);
    simulation.start().expect("run starts");
    let mut server = LocalMcpServer::default();
    let (_, trace) = server.call(
        &mut simulation,
        request(
            "redaction-run",
            "cockpit-agent",
            "simulation.unknown",
            json!({ "nested": { "apiKey": "tool-secret" } }),
        ),
    );
    assert_eq!(trace.arguments["nested"]["apiKey"], "[REDACTED]");
    assert!(!trace.arguments.to_string().contains("tool-secret"));
}

#[test]
fn open_world_controls_require_human_scope_and_preserve_owner() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("control-scope-run", scenario);
    simulation.start().expect("run starts");
    let human_id = simulation
        .snapshot
        .primary_human()
        .expect("scenario seeds one human")
        .id
        .clone();
    let mut server = LocalMcpServer::default();

    let (unauthenticated, unauthenticated_trace) = server.call(
        &mut simulation,
        request(
            "control-scope-run",
            "cockpit-agent",
            TOOL_ADD_GOAL,
            json!({ "description": "observe the next safe opening", "priority": 5 }),
        ),
    );
    assert_eq!(
        unauthenticated
            .error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("HUMAN_SCOPE_REQUIRED")
    );
    assert!(!unauthenticated_trace.allowed);

    let mut scoped = request(
        "control-scope-run",
        "cockpit-agent",
        TOOL_WAIT_UNTIL,
        json!({ "wakeAtTick": 4 }),
    );
    scoped.human_id = Some(human_id.clone());
    let (accepted, trace) = server.call(&mut simulation, scoped);
    assert!(accepted.error.is_none(), "{accepted:?}");
    assert!(trace.side_effect);
    assert_eq!(
        server.take_control_requests(),
        vec![OpenWorldControlRequest::WaitUntil {
            human_id,
            wake_tick: 4,
        }]
    );
}
