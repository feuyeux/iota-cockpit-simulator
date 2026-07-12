use cockpit_recording::{
    CURRENT_SCHEMA_VERSION, MigrationError, migrate_recording_bytes, replay_recording,
    run_rule_agent_recording,
};
use cockpit_scenario::load_scenario;
use serde_json::Value;

fn scenario() -> cockpit_simulation_core::SimulationScenario {
    load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads")
}

/// A legacy recording that predates the versioned provenance fields is migrated
/// forward and then replays deterministically against the same scenario.
#[test]
fn legacy_recording_migrates_and_replays() {
    let recording = run_rule_agent_recording("migration-run", scenario(), 12).expect("recording");
    let expected_hash = recording.final_snapshot_hash().map(str::to_string);

    // Strip the schema/provenance fields to simulate a version-0 recording.
    let mut value = serde_json::to_value(&recording).expect("serializes");
    let object = value.as_object_mut().expect("object");
    for key in [
        "schemaVersion",
        "runtimeContractVersion",
        "worldModelVersion",
        "applicationCommit",
        "pluginHashes",
    ] {
        object.remove(key);
    }
    let legacy_bytes = serde_json::to_vec(&value).expect("legacy bytes");

    let (migrated, report) = migrate_recording_bytes(&legacy_bytes).expect("migration succeeds");
    assert_eq!(report.from_version, 0);
    assert_eq!(report.to_version, CURRENT_SCHEMA_VERSION);
    assert!(report.migrated());
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.runtime_contract_version, 1);
    assert_eq!(migrated.world_model_version, 1);
    assert_eq!(migrated.ticks.len(), recording.ticks.len());

    // The migrated recording is now replayable and reproduces the same final
    // snapshot hash.
    let replay = replay_recording("migration-replay", scenario(), &migrated).expect("replays");
    assert_eq!(replay.final_snapshot_hash().map(str::to_string), expected_hash);
}

/// A current-schema recording round-trips unchanged through migration.
#[test]
fn current_recording_migration_is_noop() {
    let recording = run_rule_agent_recording("noop-run", scenario(), 6).expect("recording");
    let bytes = serde_json::to_vec(&recording).expect("bytes");
    let (migrated, report) = migrate_recording_bytes(&bytes).expect("migration succeeds");
    assert!(!report.migrated());
    assert_eq!(migrated.run_id, recording.run_id);
    assert_eq!(
        migrated.final_snapshot_hash(),
        recording.final_snapshot_hash()
    );
}

/// A recording newer than this build is rejected rather than silently accepted.
#[test]
fn newer_recording_is_rejected() {
    let recording = run_rule_agent_recording("future-run", scenario(), 4).expect("recording");
    let mut value = serde_json::to_value(&recording).expect("serializes");
    value["schemaVersion"] = Value::from(CURRENT_SCHEMA_VERSION + 1);
    let bytes = serde_json::to_vec(&value).expect("bytes");
    let error = migrate_recording_bytes(&bytes).expect_err("rejected");
    assert!(matches!(error, MigrationError::TooNew { .. }));
}
