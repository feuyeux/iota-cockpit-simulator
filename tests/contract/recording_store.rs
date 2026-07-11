use cockpit_recording::{RecordingStore, run_rule_agent_recording};
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
