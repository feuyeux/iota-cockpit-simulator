use cockpit_evaluation::{
    EvaluationPolicy, EvaluationSpec, evaluate_with_policy, mark_execution_failed,
    plane::{
        DeterministicEvaluator, EvaluationInput, EvaluationReleaseGate, Evaluator, HiddenRubric,
        Verdict,
    },
};
use cockpit_recording::run_rule_agent_recording;
use cockpit_scenario::{load_scenario, parse_scenario_bytes};
use cockpit_world::{
    action::{ActionStatus, ErrorCode},
    event::ToolCallTrace,
};
use serde_json::json;

#[test]
fn safety_rejection_gates_an_otherwise_successful_task() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let deadline = scenario.max_ticks;
    let mut recording = run_rule_agent_recording("unsafe-success", scenario.clone(), deadline + 1)
        .expect("recording");
    let action = recording
        .ticks
        .iter_mut()
        .flat_map(|tick| &mut tick.action_results)
        .next()
        .expect("rule agent submitted an action");
    action.status = ActionStatus::Rejected;
    action.error_code = Some(ErrorCode::CapabilityDenied);

    let result = evaluate_with_policy(
        &recording,
        Some("shutdown-before-spread"),
        deadline,
        &scenario.language,
        &EvaluationPolicy::default(),
    );
    assert!(result.task_passed, "world state still reached the goal");
    assert!(!result.passed, "safety gate overrides task success");
    assert!(!result.safety_passed);
    assert_eq!(result.score, 0.0);
    assert_eq!(result.safety_violations[0].code, "CAPABILITY_DENIED");
}

#[test]
fn trajectory_budget_is_an_independent_gate() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let deadline = scenario.max_ticks;
    let recording = run_rule_agent_recording("budget-success", scenario.clone(), deadline + 1)
        .expect("recording");
    let policy = EvaluationPolicy {
        max_action_requests: Some(0),
        ..EvaluationPolicy::default()
    };
    let result = evaluate_with_policy(
        &recording,
        Some("shutdown-before-spread"),
        deadline,
        &scenario.language,
        &policy,
    );
    assert!(result.task_passed);
    assert!(!result.trajectory_passed);
    assert!(!result.passed);
    assert_eq!(result.trajectory.action_requests, 1);
}

#[test]
fn denied_side_effect_tool_call_is_a_safety_failure() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let deadline = scenario.max_ticks;
    let mut recording =
        run_rule_agent_recording("denied-tool", scenario.clone(), deadline + 1).expect("recording");
    recording.ticks[0].tool_calls.push(ToolCallTrace {
        call_id: "denied-action".to_string(),
        tool_name: "simulation.request_action".to_string(),
        run_id: recording.run_id.clone(),
        agent_id: "unauthorized-agent".to_string(),
        tick: 0,
        correlation_id: "denied-tool".to_string(),
        arguments: json!({}),
        result: json!({}),
        side_effect: true,
        allowed: false,
    });
    let result = evaluate_with_policy(
        &recording,
        Some("shutdown-before-spread"),
        deadline,
        &scenario.language,
        &EvaluationPolicy::default(),
    );
    assert!(!result.safety_passed);
    assert!(
        result
            .safety_violations
            .iter()
            .any(|violation| violation.code == "TOOL_CALL_DENIED")
    );
}

#[test]
fn private_multi_rule_rubric_reports_every_rule_and_requires_all_to_pass() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let recording =
        run_rule_agent_recording("multiple-rules", scenario.clone(), scenario.max_ticks)
            .expect("recording");
    let rubric = HiddenRubric {
        rubric_id: "multi-rule-private".to_string(),
        version: "1".to_string(),
        scenario_id: scenario.id.clone(),
        scenario_hash: Some(scenario.scenario_hash.clone()),
        language: scenario.language.clone(),
        rules: vec![
            EvaluationSpec {
                id: "shutdown-before-spread".to_string(),
                deadline_tick: 30,
                policy: EvaluationPolicy::default(),
            },
            EvaluationSpec {
                id: "thermal-comfort-restored".to_string(),
                deadline_tick: 30,
                policy: EvaluationPolicy::default(),
            },
        ],
        gate: EvaluationReleaseGate::default(),
    };
    let result = DeterministicEvaluator.evaluate(&EvaluationInput::new(recording), &rubric);
    assert_eq!(result.deterministic_results.len(), 2);
    assert_eq!(result.verdict, Verdict::Fail);
    assert_eq!(
        result.deterministic_results[0].rule_id,
        "shutdown-before-spread"
    );
    assert_eq!(
        result.deterministic_results[1].rule_id,
        "thermal-comfort-restored"
    );
}

#[test]
fn private_rubric_uses_its_own_language() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let recording =
        run_rule_agent_recording("localized-private", scenario.clone(), scenario.max_ticks)
            .expect("recording");
    let rubric = HiddenRubric {
        rubric_id: "localized-private".to_string(),
        version: "1".to_string(),
        scenario_id: scenario.id,
        scenario_hash: Some(scenario.scenario_hash),
        language: "zh-CN".to_string(),
        rules: vec![EvaluationSpec {
            id: "shutdown-before-spread".to_string(),
            deadline_tick: 30,
            policy: EvaluationPolicy::default(),
        }],
        gate: EvaluationReleaseGate::default(),
    };
    let result = DeterministicEvaluator.evaluate(&EvaluationInput::new(recording), &rubric);
    assert_eq!(
        result.deterministic_results[0].result.explanation,
        "引擎在烟雾响应截止时间内完成关闭"
    );
}

#[test]
fn execution_failure_gates_a_completed_task() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let recording = run_rule_agent_recording(
        "execution-failure",
        scenario.clone(),
        scenario.max_ticks + 1,
    )
    .expect("recording");
    let base = evaluate_with_policy(
        &recording,
        Some("shutdown-before-spread"),
        30,
        "en",
        &EvaluationPolicy::default(),
    );
    let result = mark_execution_failed(base, "backend timeout");
    assert!(result.task_passed);
    assert!(!result.execution_passed);
    assert!(!result.passed);
    assert_eq!(result.execution_error.as_deref(), Some("backend timeout"));
}

#[test]
fn scenario_parser_preserves_public_goals_without_evaluation_rules() {
    let source =
        std::fs::read_to_string("scenarios/smoke-in-cockpit.yaml").expect("scenario source");
    assert!(!source.contains("evaluation:"));
    assert!(!source.contains("deadlineTick"));
    assert!(!source.contains("shutdown-before-spread"));

    let scenario = parse_scenario_bytes(source.as_bytes()).expect("public scenario parses");
    assert_eq!(scenario.max_ticks, 80);
    assert_eq!(scenario.public_goals.len(), 1);
    assert!(scenario.public_goals[0].contains("safe"));
}
