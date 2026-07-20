use std::path::PathBuf;

use cockpit_simulator::{
    SimulatorHandler,
    ipc::proto::{IPC_VERSION, SimulatorCommand, SimulatorRequest},
};
use serde_json::Value;

fn request(command: SimulatorCommand) -> SimulatorRequest {
    SimulatorRequest {
        version: IPC_VERSION,
        session_token: "restart-token".to_string(),
        correlation_id: "restart-correlation".to_string(),
        command,
    }
}

#[test]
fn persistent_handler_restores_snapshot_and_event_cursor_after_restart() {
    let database =
        std::env::temp_dir().join(format!("cockpit-restart-{}.sqlite", uuid::Uuid::new_v4()));
    let database_path = database.to_string_lossy().to_string();
    let mut first =
        SimulatorHandler::new_persistent("restart-token", &database_path).expect("first handler");
    assert!(
        first
            .dispatch(request(SimulatorCommand::CreateSimulationRun {
                path: "scenarios/smoke-in-cockpit.yaml".to_string(),
            }))
            .ok
    );
    assert!(
        first
            .dispatch(request(SimulatorCommand::StartSimulation))
            .ok
    );
    for _ in 0..10 {
        assert!(first.dispatch(request(SimulatorCommand::StepSimulation)).ok);
    }
    let snapshot_before = first.dispatch(request(SimulatorCommand::GetSimulationSnapshot));
    let tick_before = snapshot_before
        .result
        .as_ref()
        .and_then(|value| value.get("tick"))
        .and_then(Value::as_u64)
        .expect("snapshot tick");
    drop(first);

    let mut second =
        SimulatorHandler::new_persistent("restart-token", &database_path).expect("second handler");
    let resumed = second.dispatch(request(SimulatorCommand::ResumeSimulation {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        run_id: "run-smoke-in-cockpit".to_string(),
    }));
    assert!(resumed.ok, "{resumed:?}");
    assert_eq!(
        resumed
            .result
            .as_ref()
            .and_then(|value| value.get("tick"))
            .and_then(Value::as_u64),
        Some(tick_before)
    );

    let events = second.dispatch(request(SimulatorCommand::GetSimulationEvents {
        cursor: Some(0),
    }));
    assert!(events.ok, "{events:?}");
    let count = events
        .result
        .as_ref()
        .and_then(|value| value.get("events"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    assert!(count > 0);

    let _ = std::fs::remove_file(&database);
    let payloads = PathBuf::from(format!("{database_path}.payloads"));
    let _ = std::fs::remove_dir_all(payloads);
}

#[test]
fn persistent_handler_preserves_open_world_checkpoint_after_restart() {
    let database = std::env::temp_dir().join(format!(
        "cockpit-live-restart-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let database_path = database.to_string_lossy().to_string();
    let mut first =
        SimulatorHandler::new_persistent("restart-token", &database_path).expect("first handler");
    assert!(
        first
            .dispatch(request(SimulatorCommand::CreateSimulationRun {
                path: "scenarios/smoke-in-cockpit.yaml".to_string(),
            }))
            .ok
    );
    let goal = first.dispatch(request(SimulatorCommand::AddAgentGoal {
        agent_id: "pilot-1".to_string(),
        description: "preserve occupant safety after restart".to_string(),
        priority: 10,
    }));
    assert!(goal.ok, "{goal:?}");
    drop(first);

    let store = cockpit_recording::RecordingStore::open_read_only(&database_path)
        .expect("recording store reopens after handler restart");
    let recording = store
        .load("run-smoke-in-cockpit")
        .expect("persisted recording loads");
    let checkpoint = recording
        .open_world_checkpoint
        .as_ref()
        .expect("open-world checkpoint persisted");
    let pilot = checkpoint
        .runtime
        .sessions
        .get("pilot-1")
        .expect("pilot runtime persisted");
    assert!(
        pilot
            .goals
            .iter()
            .any(|goal| goal.description == "preserve occupant safety after restart")
    );

    let mut second =
        SimulatorHandler::new_persistent("restart-token", &database_path).expect("second handler");
    let resumed = second.dispatch(request(SimulatorCommand::ResumeSimulation {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        run_id: "run-smoke-in-cockpit".to_string(),
    }));
    assert!(resumed.ok, "{resumed:?}");

    let _ = std::fs::remove_file(&database);
    let payloads = PathBuf::from(format!("{database_path}.payloads"));
    let _ = std::fs::remove_dir_all(payloads);
}
