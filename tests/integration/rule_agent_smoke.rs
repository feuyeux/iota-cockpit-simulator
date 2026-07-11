use cockpit_evaluation::evaluate_smoke_shutdown;
use cockpit_recording::run_rule_agent_recording;
use cockpit_scenario::load_scenario;

#[test]
fn rule_agent_uses_mcp_boundary_to_shutdown_engine() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let deadline = scenario.shutdown_deadline_ticks;
    let recording =
        run_rule_agent_recording("rule-agent-run", scenario, 80).expect("run completes");
    let evaluation = evaluate_smoke_shutdown(&recording, deadline);

    assert!(evaluation.passed, "{evaluation:?}");
    assert!(recording.ticks.iter().any(|tick| {
        tick.tool_calls
            .iter()
            .any(|call| call.tool_name == "simulation.request_action")
    }));
    assert!(recording.ticks.iter().all(|tick| {
        tick.tool_calls.iter().all(|call| {
            let serialized = call.result.to_string();
            !serialized.contains("smokeDensity") && !serialized.contains("fireActive")
        })
    }));
}
