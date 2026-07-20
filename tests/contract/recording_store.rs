use cockpit_agent::{HumanDecision, HumanTurnEvidence, OpenWorldCheckpoint, OpenWorldRuntime};
use cockpit_recording::{
    AsyncRecordingSink, Recording, RecordingQueueOutcome, RecordingQueuePolicy, RecordingStore,
    RunProvenance, run_rule_agent_recording,
};
use cockpit_scenario::load_scenario;

#[test]
fn sqlite_recording_round_trip_preserves_tick_evidence() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let recording = run_rule_agent_recording("sqlite-run", scenario, 12).expect("run completes");
    let mut store = RecordingStore::in_memory().expect("store opens");
    store.save(&recording).expect("recording saves");
    let restored = store.load("sqlite-run").expect("recording loads");

    assert_eq!(restored.scenario_hash, recording.scenario_hash);
    assert_eq!(restored.ticks.len(), recording.ticks.len());
    assert_eq!(
        restored.final_snapshot_hash(),
        recording.final_snapshot_hash()
    );
    assert!(
        restored
            .ticks
            .iter()
            .any(|tick| !tick.tool_calls.is_empty())
    );
}

#[test]
fn sqlite_recording_round_trip_preserves_live_human_turns() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut recording = Recording::new("sqlite-live-run", &scenario);
    let mut runtime = OpenWorldRuntime::default();
    runtime.ensure_agent("pilot-1", "protect occupants", 0);
    let world = cockpit_world::Simulation::new("sqlite-live-run", scenario.clone());
    recording.open_world_checkpoint = Some(OpenWorldCheckpoint::capture(&world.snapshot, &runtime));
    recording.provenance = RunProvenance {
        suite_id: Some("release-suite".to_string()),
        split: Some("hiddenRelease".to_string()),
        backend: Some("iota-core-acp".to_string()),
        ..RunProvenance::default()
    };
    recording.push_human_turns(vec![HumanTurnEvidence {
        human_id: "pilot-1".to_string(),
        decision: HumanDecision {
            narrative: "watched the engine panel".to_string(),
            utterance: Some("status check".to_string()),
            ..HumanDecision::default()
        },
        tool_calls: Vec::new(),
        latency_ms: None,
    }]);

    let mut store = RecordingStore::in_memory().expect("store opens");
    store.save(&recording).expect("recording saves");
    let restored = store.load("sqlite-live-run").expect("recording loads");

    assert_eq!(restored.human_turns.len(), 1);
    assert_eq!(restored.provenance, recording.provenance);
    assert_eq!(
        restored.open_world_checkpoint,
        recording.open_world_checkpoint
    );
    assert_eq!(
        restored
            .open_world_checkpoint
            .as_ref()
            .expect("checkpoint restored")
            .runtime
            .sessions
            .len(),
        1
    );
    assert_eq!(restored.human_turns[0][0].human_id, "pilot-1");
    assert_eq!(restored.human_turns[0][0].decision.narrative, "[REDACTED]");
    assert_eq!(
        restored.human_turns[0][0].decision.utterance.as_deref(),
        Some("[REDACTED]")
    );
}

#[test]
fn sustained_async_overload_triggers_bounded_queue_policy() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let recording = run_rule_agent_recording("overload-run", scenario, 16).expect("run completes");
    assert!(recording.ticks.len() >= 4, "need several steps to overflow");

    // A slow async consumer that never makes progress while the producer keeps
    // pushing: the bounded queue must reject once capacity is exceeded.
    let mut sink = AsyncRecordingSink::new(2, RecordingQueuePolicy::FailRun);
    let mut outcomes = Vec::new();
    for step in recording.ticks.iter().cloned() {
        outcomes.push(sink.push(step));
    }

    assert_eq!(outcomes[0], RecordingQueueOutcome::Enqueued);
    assert_eq!(outcomes[1], RecordingQueueOutcome::Enqueued);
    assert!(
        outcomes[2..]
            .iter()
            .all(|outcome| *outcome == RecordingQueueOutcome::Failed),
        "sustained overload with a lagging consumer must fail closed: {outcomes:?}"
    );
    let health = sink.health();
    assert_eq!(health.capacity, 2);
    assert_eq!(health.enqueued, 2);
    assert!(health.rejected >= 1, "overflow is observable in health");
}

#[test]
fn async_consumer_catching_up_commits_every_step() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let recording = run_rule_agent_recording("drain-run", scenario, 10).expect("run completes");

    // Consumer keeps pace: drain one step after each push so the queue never
    // overflows and every step is eventually committed.
    let mut sink = AsyncRecordingSink::new(2, RecordingQueuePolicy::FailRun);
    for step in recording.ticks.iter().cloned() {
        assert_eq!(sink.push(step), RecordingQueueOutcome::Enqueued);
        sink.drain_one();
    }
    sink.drain_all();
    assert_eq!(
        sink.committed().len(),
        recording.ticks.len(),
        "a consumer that keeps pace commits every step"
    );
}
