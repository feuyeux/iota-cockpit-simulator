use cockpit_runner::ipc::{
    RunnerHandler,
    proto::{IPC_VERSION, RunnerCommand, RunnerRequest},
};
use serde_json::Value;

fn request(command: RunnerCommand) -> RunnerRequest {
    RunnerRequest {
        version: IPC_VERSION,
        session_token: "session-1".to_string(),
        correlation_id: "contract-correlation".to_string(),
        command,
    }
}

#[test]
fn runner_requires_version_and_session_token() {
    let mut handler = RunnerHandler::new("session-1");
    let mut invalid = request(RunnerCommand::GetSimulationSnapshot);
    invalid.version = IPC_VERSION + 1;
    let response = handler.dispatch(invalid);
    assert_eq!(
        response.error.as_ref().map(|error| error.code.as_str()),
        Some("IPC_VERSION_UNSUPPORTED")
    );

    let mut unauthorized = request(RunnerCommand::GetSimulationSnapshot);
    unauthorized.session_token = "wrong".to_string();
    let response = handler.dispatch(unauthorized);
    assert_eq!(
        response.error.as_ref().map(|error| error.code.as_str()),
        Some("SESSION_UNAUTHORIZED")
    );
}

#[test]
fn runner_step_emits_snapshot_trace_evaluation_and_cursored_events() {
    let mut handler = RunnerHandler::new("session-1");
    let response = handler.dispatch(request(RunnerCommand::CreateSimulationRun {
        path: "scenarios/smoke-in-cockpit.yaml".to_string(),
    }));
    assert!(response.ok, "{response:?}");
    let response = handler.dispatch(request(RunnerCommand::StartSimulation));
    assert!(response.ok, "{response:?}");

    for _ in 0..10 {
        let response = handler.dispatch(request(RunnerCommand::StepSimulation));
        assert!(response.ok, "{response:?}");
    }

    let response = handler.dispatch(request(RunnerCommand::GetSimulationEvents {
        cursor: Some(0),
    }));
    assert!(response.ok, "{response:?}");
    let events = response
        .result
        .expect("event result")
        .get("events")
        .cloned()
        .expect("events");
    let events = events.as_array().expect("event array");
    assert!(
        events.iter().any(|event| event.get("type")
            == Some(&Value::String("SimulationTickCommitted".to_string())))
    );
    assert!(
        events.iter().any(
            |event| event.get("type") == Some(&Value::String("SimulationToolCall".to_string()))
        )
    );
    assert!(events.iter().any(|event| event.get("type")
        == Some(&Value::String("SimulationEvaluationUpdated".to_string()))));

    let cursor = events
        .last()
        .and_then(|event| event.get("cursor"))
        .and_then(Value::as_u64)
        .expect("cursor");
    let response = handler.dispatch(request(RunnerCommand::GetSimulationEvents {
        cursor: Some(cursor),
    }));
    assert!(response.ok, "{response:?}");
    assert_eq!(
        response
            .result
            .as_ref()
            .and_then(|result| result.get("events"))
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
}
