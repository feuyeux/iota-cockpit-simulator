use cockpit_agent::{
    HumanToolCall, HumanToolExchange, HumanTurnContext, TOOL_GET_TURN_CONTEXT, ToolResponse,
    acp_adapter::IotaCoreAcpAdapter, iota_core_adapter::IotaCoreAdapter,
};
use cockpit_scenario::load_scenario;
use cockpit_world::Simulation;
use serde_json::{Value, json};

fn prompt_tool_names(prompt: &str) -> Vec<String> {
    let (_, tools) = prompt
        .split_once("Available simulation tools (JSON definitions):\n")
        .expect("prompt contains tool definitions");
    let (tools, _) = tools
        .split_once("\n\nTool exchanges completed")
        .expect("tool definitions end before exchanges");
    serde_json::from_str::<Vec<Value>>(tools)
        .expect("tool definitions are JSON")
        .into_iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn context_for_primary_human(simulation: &Simulation) -> HumanTurnContext {
    let human = simulation
        .snapshot
        .primary_human()
        .expect("scenario seeds one human")
        .clone();
    HumanTurnContext {
        human_id: human.id.clone(),
        persona: human.persona.clone(),
        needs: human.needs,
        goal: human.goal.clone(),
        delivered_perception: human.short_term_memory.clone(),
        long_term_memory: human.long_term_memory.clone(),
        action_capabilities: human.action_capabilities.clone(),
        tool_history: Vec::new(),
        round: 0,
        language: simulation.scenario.language.clone(),
    }
}

#[test]
fn acp_prompt_starts_without_eager_observation_and_exposes_tools() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let simulation = Simulation::new("acp-contract-run", scenario);
    let skill = IotaCoreAdapter::new(env!("CARGO_MANIFEST_DIR"))
        .load_cockpit_skill()
        .expect("skill loads");
    let context = context_for_primary_human(&simulation);
    let prompt = IotaCoreAcpAdapter::build_prompt(&context, &skill);

    assert!(prompt.contains("Never request or infer Ground Truth"));
    assert!(prompt.contains(&context.persona.name));
    assert!(prompt.contains("Personality (Big Five"));
    assert!(prompt.contains("simulation.get_turn_context"));
    assert_eq!(prompt_tool_names(&prompt), vec![TOOL_GET_TURN_CONTEXT]);
    assert!(prompt.contains("no complete Observation is injected"));

    // Neither authorized perception nor Ground Truth is eagerly injected.
    assert!(!prompt.contains("visibleEntities"));
    assert!(!prompt.contains("smokeDensity"));
    assert!(!prompt.contains("fireActive"));
}

#[test]
fn acp_prompt_exposes_authorized_tools_without_leaking_the_benchmark_action() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let simulation = Simulation::new("acp-required-action", scenario);
    let skill = IotaCoreAdapter::new(env!("CARGO_MANIFEST_DIR"))
        .load_cockpit_skill()
        .expect("skill loads");
    let mut context = context_for_primary_human(&simulation);
    context.tool_history.push(HumanToolExchange {
        call_id: "turn-context".to_string(),
        call: HumanToolCall {
            tool: TOOL_GET_TURN_CONTEXT.to_string(),
            arguments: json!({}),
        },
        response: ToolResponse {
            run_id: "acp-required-action".to_string(),
            tick: 0,
            correlation_id: "turn-context-correlation".to_string(),
            result: json!({ "stateVersion": 0 }),
            error: None,
        },
    });

    let prompt = IotaCoreAcpAdapter::build_prompt(&context, &skill);

    assert!(prompt.contains("- engineShutdown -> engine-1"));
    assert!(prompt_tool_names(&prompt).contains(&"simulation.request_action".to_string()));
    assert!(!prompt.contains("SmokeDetected"));
    assert!(!prompt.contains("SmokeDetected: engineShutdown -> engine-1"));
    assert!(!prompt.contains("include every listed action in the actions array this turn"));
}
