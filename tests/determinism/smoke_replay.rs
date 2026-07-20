use cockpit_evaluation::evaluate_smoke_shutdown;
use cockpit_recording::{
    Recording, replay_recording, run_rule_agent_recording, run_scripted_recording,
};
use cockpit_scenario::load_scenario;
use cockpit_world::{
    ActionRequest, Simulation, StateDiff, capability::CapabilityCatalog, resolve_action,
};
use serde_json::json;

#[test]
fn smoke_scenario_records_replays_and_evaluates_deterministically() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let deadline = scenario.max_ticks;

    let first = run_scripted_recording("smoke-run-1", scenario.clone(), 80).expect("first run");
    let second = run_scripted_recording("smoke-run-2", scenario.clone(), 80).expect("second run");
    let replay = replay_recording("smoke-replay-1", scenario, &first).expect("replay");

    assert_eq!(first.final_snapshot_hash(), second.final_snapshot_hash());
    assert_eq!(first.final_snapshot_hash(), replay.final_snapshot_hash());

    let evaluation = evaluate_smoke_shutdown(&first, deadline);
    assert!(evaluation.passed, "{evaluation:?}");

    let leaked_ground_truth = first.ticks.iter().any(|tick| {
        serde_json::to_value(&tick.observation)
            .unwrap()
            .get("smokeDensity")
            .is_some()
    });
    assert!(
        !leaked_ground_truth,
        "observation leaked ground truth smoke density"
    );

    let smoke_tick = first
        .ticks
        .iter()
        .flat_map(|tick| &tick.events)
        .find(|event| event.event_type == "SmokeDetected")
        .expect("smoke detected")
        .tick;
    let shutdown_tick = first
        .ticks
        .iter()
        .flat_map(|tick| &tick.events)
        .find(|event| event.event_type == "EngineShutdown")
        .expect("engine shutdown")
        .tick;
    assert!(shutdown_tick <= smoke_tick + deadline);
}

#[test]
fn smoke_shutdown_is_resolved_by_the_generic_effect_kernel() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let simulation = Simulation::new("effect-plan-run", scenario);
    let catalog = CapabilityCatalog::load_default();
    let request = ActionRequest {
        request_id: "shutdown-effect".to_string(),
        agent_id: "cockpit-agent".to_string(),
        target: "engine-1".to_string(),
        capability_id: "engine.shutdown".to_string(),
        expected_state_version: 0,
        expires_at_tick: 3,
        correlation_id: "shutdown-effect-corr".to_string(),
    };

    let plan =
        resolve_action(&catalog, &simulation.snapshot, &request).expect("effect plan resolves");

    assert_eq!(plan.resolver, "device-capability+combustion");
    assert_eq!(plan.operations.len(), 3);
    assert_eq!(plan.events[0].event_type, "ActionApplied");
    assert_eq!(plan.events[1].event_type, "EngineShutdown");
}

#[test]
fn committed_state_diffs_are_audited_and_replayed_deterministically() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("state-diff-run", scenario.clone());
    simulation.start().expect("run starts");
    let diff = StateDiff {
        source_id: "smoke-plugin".to_string(),
        entity_id: "cabin".to_string(),
        component_path: "environment.visibility".to_string(),
        value: json!(0.4),
        expected_state_version: 0,
    };
    let step = simulation
        .step_with_state_diffs(vec![diff])
        .expect("state diff commits");
    assert_eq!(simulation.snapshot.environment.visibility, 0.4);
    assert!(
        step.events
            .iter()
            .any(|event| event.event_type == "StateDiffApplied")
    );

    let mut recording = Recording::new("state-diff-run", &scenario);
    recording.push(step);
    let replay = replay_recording("state-diff-replay", scenario, &recording).expect("replay");
    assert_eq!(
        recording.final_snapshot_hash(),
        replay.final_snapshot_hash()
    );
}

#[test]
fn cockpit_system_state_is_included_in_deterministic_replay() {
    let scenario =
        load_scenario("scenarios/heatwave-thermal-comfort.yaml").expect("scenario loads");
    let recording =
        run_rule_agent_recording("domain-state-run", scenario.clone(), 30).expect("rule run");
    let replay = replay_recording("domain-state-replay", scenario, &recording).expect("replay");

    assert_eq!(
        recording.final_snapshot_hash(),
        replay.final_snapshot_hash()
    );
    assert!(
        recording
            .ticks
            .iter()
            .flat_map(|tick| &tick.events)
            .any(|event| event.event_type == "ThermalComfortRestored")
    );
    assert!(
        replay
            .ticks
            .iter()
            .flat_map(|tick| &tick.events)
            .any(|event| event.event_type == "ThermalComfortRestored")
    );
}

/// Every bundled benchmark scenario, driven by the deterministic `RuleAgent`,
/// must record and replay to the identical final snapshot hash. Coverage
/// previously stopped at the smoke scenario (via
/// `smoke_scenario_records_replays_and_evaluates_deterministically`) and the
/// heatwave scenario (via
/// `cockpit_system_state_is_included_in_deterministic_replay`); the other
/// eight benchmark scenarios had no determinism/replay coverage at all, so a
/// scenario-specific replay regression (e.g. a `cockpit_systems` field
/// excluded from the hash, or a non-deterministic domain action ordering)
/// could ship unnoticed. This parameterizes the same record/replay/hash
/// check across the full ten-scenario catalog.
const ALL_BENCHMARK_SCENARIOS: &[&str] = &[
    "scenarios/smoke-in-cockpit.yaml",
    "scenarios/heatwave-thermal-comfort.yaml",
    "scenarios/winter-defog-visibility.yaml",
    "scenarios/driver-fatigue-guardian.yaml",
    "scenarios/child-left-behind.yaml",
    "scenarios/medical-emergency.yaml",
    "scenarios/voice-privacy-conflict.yaml",
    "scenarios/ev-range-anxiety.yaml",
    "scenarios/adas-takeover-construction.yaml",
    "scenarios/cybersecurity-anomalous-control.yaml",
];

#[test]
fn every_benchmark_scenario_replays_to_an_identical_snapshot_hash() {
    for path in ALL_BENCHMARK_SCENARIOS {
        let scenario = load_scenario(path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let ticks = scenario.max_ticks + 1;

        let first = run_rule_agent_recording(
            format!("determinism-{}-1", scenario.id),
            scenario.clone(),
            ticks,
        )
        .unwrap_or_else(|error| panic!("{path}: first run failed: {error}"));
        let second = run_rule_agent_recording(
            format!("determinism-{}-2", scenario.id),
            scenario.clone(),
            ticks,
        )
        .unwrap_or_else(|error| panic!("{path}: second run failed: {error}"));
        assert_eq!(
            first.final_snapshot_hash(),
            second.final_snapshot_hash(),
            "{path}: two independent RuleAgent runs of the same scenario diverged"
        );

        let replay = replay_recording(
            format!("determinism-{}-replay", scenario.id),
            scenario,
            &first,
        )
        .unwrap_or_else(|error| panic!("{path}: replay failed: {error}"));
        assert_eq!(
            first.final_snapshot_hash(),
            replay.final_snapshot_hash(),
            "{path}: replay diverged from the original recording"
        );
    }
}
