use cockpit_scenario::load_scenario;
use cockpit_simulation_core::{
    ConflictPolicy, InfluenceOp, InfluenceRule, InfluenceSchedule, Simulation, SimulationScenario,
};

fn base_scenario() -> SimulationScenario {
    load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads")
}

fn rule(id: &str, tick: u64, priority: i32, op: InfluenceOp) -> InfluenceRule {
    InfluenceRule {
        rule_id: id.to_string(),
        rule_version: 1,
        schedule: InfluenceSchedule::AtTick { tick },
        entity_id: "pilot-1".to_string(),
        component_path: "pilot.attention".to_string(),
        op,
        priority,
    }
}

fn run(scenario: SimulationScenario, ticks: u64) -> Vec<String> {
    let mut simulation = Simulation::new("influence-run", scenario);
    simulation.start().expect("starts");
    let mut event_types = Vec::new();
    for _ in 0..ticks {
        let step = simulation.step_without_agent().expect("step");
        for event in step.events {
            event_types.push(event.event_type);
        }
    }
    event_types
}

#[test]
fn scheduled_influence_applies_and_emits_event() {
    let mut scenario = base_scenario();
    scenario.influences = vec![rule("set-attention", 2, 0, InfluenceOp::Set(0.25))];

    let mut simulation = Simulation::new("influence-apply", scenario);
    simulation.start().expect("starts");
    // Ticks 0 and 1: not due.
    simulation.step_without_agent().expect("tick 0");
    simulation.step_without_agent().expect("tick 1");
    let step = simulation.step_without_agent().expect("tick 2");
    assert!(
        step.events
            .iter()
            .any(|event| event.event_type == "InfluenceApplied"),
        "due rule emits an InfluenceApplied event"
    );
    assert!((simulation.snapshot.pilot.attention - 0.25).abs() < 1e-9);
}

#[test]
fn conflicting_rules_are_rejected_under_reject_policy() {
    let mut scenario = base_scenario();
    scenario.conflict_policy = ConflictPolicy::RejectConflicting;
    scenario.influences = vec![
        rule("a", 1, 0, InfluenceOp::Set(0.2)),
        rule("b", 1, 5, InfluenceOp::Set(0.8)),
    ];
    let events = run(scenario, 2);
    assert!(
        events.iter().any(|event| event == "InfluenceRejected"),
        "conflicting rules produce rejection evidence"
    );
    assert!(
        !events.iter().any(|event| event == "InfluenceApplied"),
        "no conflicting rule is applied under the reject policy"
    );
}

#[test]
fn highest_priority_wins_is_deterministic() {
    let build = || {
        let mut scenario = base_scenario();
        scenario.conflict_policy = ConflictPolicy::HighestPriorityWins;
        scenario.influences = vec![
            rule("a", 1, 1, InfluenceOp::Set(0.2)),
            rule("b", 1, 9, InfluenceOp::Set(0.8)),
        ];
        scenario
    };

    let mut first = Simulation::new("influence-arb-1", build());
    first.start().expect("starts");
    first.step_without_agent().expect("tick 0");
    first.step_without_agent().expect("tick 1");

    let mut second = Simulation::new("influence-arb-2", build());
    second.start().expect("starts");
    second.step_without_agent().expect("tick 0");
    second.step_without_agent().expect("tick 1");

    // Highest-priority rule "b" (0.8) wins in both runs.
    assert!((first.snapshot.pilot.attention - 0.8).abs() < 1e-9);
    assert_eq!(
        first.snapshot.pilot.attention, second.snapshot.pilot.attention,
        "arbitration is deterministic across runs"
    );
}

#[test]
fn scenario_without_influences_is_unchanged() {
    // A scenario with no influences must not emit any influence events, so
    // existing replay hashes are unaffected.
    let events = run(base_scenario(), 10);
    assert!(
        !events.iter().any(|event| event.starts_with("Influence")),
        "no influence events without configured rules"
    );
}
