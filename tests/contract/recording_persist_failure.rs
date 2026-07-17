//! Regression coverage for a transient recording-persistence failure (disk
//! full, permission denied, SQLite lock contention) during tick commit.
//!
//! Before the fix, `RunnerHandler::step`/`step_live` called
//! `self.persist_recording()?` *before* writing the locally-owned
//! `Simulation` back to `self.simulation`. A storage error therefore
//! propagated through `?` immediately, returning from the handler with
//! `self.simulation` left at `None` even though the tick had already been
//! committed in memory - permanently stranding the run. Every subsequent
//! command would then fail with "no run in progress".
#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use cockpit_runner::{
        RunnerHandler,
        ipc::proto::{IPC_VERSION, RunnerCommand, RunnerRequest},
    };
    use serde_json::Value;

    fn request(command: RunnerCommand) -> RunnerRequest {
        RunnerRequest {
            version: IPC_VERSION,
            session_token: "persist-fail-token".to_string(),
            correlation_id: "persist-fail-correlation".to_string(),
            command,
        }
    }

    #[test]
    fn a_transient_persist_failure_does_not_strand_the_run() {
        let database = std::env::temp_dir().join(format!(
            "cockpit-persist-fail-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let database_path = database.to_string_lossy().to_string();
        let mut handler =
            RunnerHandler::new_persistent("persist-fail-token", &database_path).expect("handler");

        assert!(
            handler
                .dispatch(request(RunnerCommand::CreateSimulationRun {
                    path: "scenarios/smoke-in-cockpit.yaml".to_string(),
                }))
                .ok
        );
        assert!(handler.dispatch(request(RunnerCommand::StartSimulation)).ok);
        // At least one successful tick so the payload directory exists
        // before permissions are removed below.
        assert!(handler.dispatch(request(RunnerCommand::StepSimulation)).ok);

        let payload_root = database.with_extension("payloads");
        assert!(
            payload_root.exists(),
            "payload root must exist after a committed tick"
        );
        let original_permissions = fs::metadata(&payload_root)
            .expect("payload metadata")
            .permissions();
        fs::set_permissions(&payload_root, fs::Permissions::from_mode(0o444))
            .expect("make payload root read-only");

        // The step itself must still succeed from the caller's perspective:
        // the tick commit is a Simulation-Core-owned fact, and persistence
        // is best-effort. The response must report `ok: true`, not a hard
        // failure that discards the run.
        let step_during_outage = handler.dispatch(request(RunnerCommand::StepSimulation));
        assert!(
            step_during_outage.ok,
            "a persistence failure must not fail the step command: {step_during_outage:?}"
        );

        // Restore permissions before asserting so the test cleans up even if
        // an assertion below fails.
        fs::set_permissions(&payload_root, original_permissions)
            .expect("restore payload root permissions");

        // The critical regression check: `self.simulation` must still be
        // present. If it had been stranded, this next step would fail with
        // a "no run in progress" `NO_RUN` error instead of committing tick
        // 3.
        let step_after_outage = handler.dispatch(request(RunnerCommand::StepSimulation));
        assert!(
            step_after_outage.ok,
            "the run must remain controllable after a transient persistence failure: {step_after_outage:?}"
        );
        // Three successful StepSimulation calls (line 49, line 61, this call)
        // commit ticks 0, 1, and 2 respectively. `tick` in the response is the
        // tick being committed by THIS step (pre-increment), so the third call
        // returns 2. The earlier persist failure must not have stranded the
        // run: that path would have surfaced a `NO_RUN` error here with no
        // tick at all.
        assert_eq!(
            step_after_outage
                .result
                .as_ref()
                .and_then(|value| value.get("tick"))
                .and_then(Value::as_u64),
            Some(2),
            "ticks must keep advancing without being stranded by the earlier persist failure"
        );

        let events = handler.dispatch(request(RunnerCommand::GetSimulationEvents {
            cursor: Some(0),
        }));
        assert!(events.ok, "{events:?}");
        let event_list = events
            .result
            .as_ref()
            .and_then(|value| value.get("events"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            event_list.iter().any(|event| {
                event.get("type").and_then(Value::as_str) == Some("SimulationError")
                    && event
                        .get("error")
                        .and_then(|error| error.get("code"))
                        .and_then(Value::as_str)
                        == Some("RECORDING_PERSIST_FAILED")
            }),
            "a RECORDING_PERSIST_FAILED event must be emitted so the operator is informed: {event_list:?}"
        );

        let _ = fs::remove_file(&database);
        let _ = fs::remove_dir_all(&payload_root);
    }
}
