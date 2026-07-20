use super::*;

#[test]
fn live_run_creation_read_timeout_outlasts_the_backend_warm_up_budget() {
    // A cold Hermes warm-up runs inside CreateLiveSimulationRun and can take
    // the better part of the caller's model budget. The desktop must wait at
    // least that long plus the IPC/process-spawn margin, or it severs the
    // connection mid-warm-up and reports a spurious "simulator disconnected".
    let timeout_ms = 60_000;
    let read_timeout = live_command_read_timeout(&SimulatorCommand::CreateLiveSimulationRun {
        path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        timeout_ms,
    });
    assert_eq!(
        read_timeout,
        Duration::from_millis(timeout_ms) + LIVE_COMMAND_IPC_MARGIN
    );
    assert!(read_timeout > Duration::from_secs(5));
}

#[test]
fn live_resume_read_timeout_also_tracks_its_backend_budget() {
    let timeout_ms = 120_000;
    let read_timeout = live_command_read_timeout(&SimulatorCommand::ResumeLiveSimulation {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        run_id: "live-run-smoke".to_string(),
        timeout_ms,
    });
    assert_eq!(
        read_timeout,
        Duration::from_millis(timeout_ms) + LIVE_COMMAND_IPC_MARGIN
    );
}

#[test]
fn live_step_keeps_a_generous_fixed_ceiling() {
    assert_eq!(
        live_command_read_timeout(&SimulatorCommand::StepLiveSimulation),
        Duration::from_secs(600)
    );
}

#[test]
fn cheap_synchronous_commands_keep_the_short_read_timeout() {
    assert_eq!(
        live_command_read_timeout(&SimulatorCommand::GetSimulationSnapshot),
        Duration::from_millis(5_000)
    );
    assert_eq!(
        live_command_read_timeout(&SimulatorCommand::CreateSimulationRun {
            path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        }),
        Duration::from_millis(5_000)
    );
}

#[test]
fn slow_operation_logging_uses_a_one_second_threshold() {
    assert!(!should_log_slow_operation(
        std::time::Duration::from_millis(999)
    ));
    assert!(should_log_slow_operation(std::time::Duration::from_secs(1)));
}

#[test]
fn bundled_simulator_is_resolved_next_to_the_desktop_executable() {
    let executable = Path::new("target/release/cockpit-desktop");
    let expected = Path::new("target/release").join(if cfg!(windows) {
        "cockpit-simulator.exe"
    } else {
        "cockpit-simulator"
    });

    assert_eq!(bundled_simulator_path(executable), Some(expected));
}

#[test]
fn bundled_simulator_moves_out_of_the_test_deps_directory() {
    let executable = Path::new("target/debug/deps/cockpit-desktop-test");
    let expected = Path::new("target/debug").join(if cfg!(windows) {
        "cockpit-simulator.exe"
    } else {
        "cockpit-simulator"
    });

    assert_eq!(bundled_simulator_path(executable), Some(expected));
}

fn pong_response(seq: u64) -> SimulatorResponse {
    SimulatorResponse {
        version: IPC_VERSION,
        correlation_id: format!("heartbeat-{seq}"),
        ok: true,
        result: Some(serde_json::json!({ "pong": true, "seq": seq })),
        error: None,
    }
}

#[test]
fn heartbeat_response_requires_matching_pong_and_sequence() {
    let response = pong_response(7);
    assert!(heartbeat_response_matches(&response, 7, "heartbeat-7"));
    assert!(!heartbeat_response_matches(&response, 8, "heartbeat-8"));

    let mut wrong_protocol = pong_response(7);
    wrong_protocol.version = IPC_VERSION - 1;
    assert!(!heartbeat_response_matches(
        &wrong_protocol,
        7,
        "heartbeat-7"
    ));

    let mut missing_pong = pong_response(7);
    missing_pong.result = Some(serde_json::json!({ "seq": 7 }));
    assert!(!heartbeat_response_matches(&missing_pong, 7, "heartbeat-7"));
}

#[test]
fn durable_recording_must_match_current_simulator_position() {
    let current = serde_json::json!({ "runId": "run-1", "tick": 4 });
    assert!(validate_durable_position("run-1", 4, &current).is_ok());
    assert!(validate_durable_position("run-1", 3, &current).is_err());
    assert!(validate_durable_position("other-run", 4, &current).is_err());
}

fn state_with_workspace_root(root: &str) -> SimulatorState {
    SimulatorState::new("test-token", PathBuf::from(root))
}

#[test]
fn resolve_path_rejects_a_path_containing_a_nul_byte() {
    let state = state_with_workspace_root("/workspace/cockpit-simulator");
    let result = state.resolve_path("scenarios/smoke\0.yaml");
    assert!(result.is_err());
}

#[test]
fn resolve_path_passes_through_absolute_paths_unchanged() {
    let state = state_with_workspace_root("/workspace/cockpit-simulator");
    let absolute = if cfg!(windows) {
        "C:\\Users\\test\\scenario.yaml"
    } else {
        "/etc/scenario.yaml"
    };
    assert_eq!(state.resolve_path(absolute), Ok(absolute.to_string()));
}

#[test]
fn resolve_path_joins_a_plain_relative_path_under_the_workspace_root() {
    let state = state_with_workspace_root("/workspace/cockpit-simulator");
    let resolved = state
        .resolve_path("scenarios/smoke-in-cockpit.yaml")
        .expect("relative path within the workspace resolves");
    let expected = Path::new("/workspace/cockpit-simulator")
        .join("scenarios/smoke-in-cockpit.yaml")
        .to_string_lossy()
        .to_string();
    assert_eq!(resolved, expected);
}

#[test]
fn resolve_path_rejects_a_relative_path_that_walks_out_of_the_workspace_root() {
    let state = state_with_workspace_root("/workspace/cockpit-simulator");
    let result = state.resolve_path("../../etc/passwd");
    assert!(
        result.is_err(),
        "a relative path escaping the workspace root via .. must be rejected"
    );
}

#[test]
fn resolve_path_allows_dot_dot_segments_that_stay_inside_the_workspace_root() {
    let state = state_with_workspace_root("/workspace/cockpit-simulator");
    let resolved = state
        .resolve_path("scenarios/../scenarios/smoke-in-cockpit.yaml")
        .expect("a .. segment that nets out inside the workspace root is allowed");
    let expected = Path::new("/workspace/cockpit-simulator")
        .join("scenarios/../scenarios/smoke-in-cockpit.yaml")
        .to_string_lossy()
        .to_string();
    assert_eq!(resolved, expected);
}
