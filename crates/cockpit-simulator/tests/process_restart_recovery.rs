//! External simulator-process recovery across a real process restart.
//!
//! Spawns the actual `cockpit-simulator serve --recording-db` binary over
//! loopback, drives a few ticks, kills the process, spawns a fresh one against
//! the same database, resumes, and asserts the snapshot/event cursor recover.

use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command},
    thread::sleep,
    time::Duration,
};

use cockpit_simulator::ipc::proto::IPC_VERSION;
use serde_json::{Value, json};

const TOKEN: &str = "restart-recovery-token";

fn scenario_path() -> String {
    // The workspace scenario lives two levels up from this crate.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/smoke-in-cockpit.yaml")
        .canonicalize()
        .expect("scenario path resolves")
        .to_string_lossy()
        .to_string()
}

fn spawn_simulator(address: &SocketAddr, database: &str) -> Child {
    let child = Command::new(env!("CARGO_BIN_EXE_cockpit-simulator"))
        .args([
            "serve",
            "--bind",
            &address.to_string(),
            "--session-token",
            TOKEN,
            "--recording-db",
            database,
        ])
        .spawn()
        .expect("cockpit-simulator starts");
    // Wait for the loopback listener to accept connections.
    let connected = (0..100).any(|_| {
        let connected = TcpStream::connect_timeout(address, Duration::from_millis(50)).is_ok();
        if !connected {
            sleep(Duration::from_millis(20));
        }
        connected
    });
    assert!(connected, "simulator did not accept loopback connections");
    child
}

fn request(address: &SocketAddr, command: Value) -> Value {
    let mut stream = TcpStream::connect_timeout(address, Duration::from_millis(1_000))
        .expect("connects to simulator");
    stream
        .set_read_timeout(Some(Duration::from_millis(5_000)))
        .expect("read timeout");
    let payload = json!({
        "version": IPC_VERSION,
        "sessionToken": TOKEN,
        "correlationId": "restart-test",
        "command": command,
    });
    let mut encoded = serde_json::to_vec(&payload).expect("request serializes");
    encoded.push(b'\n');
    stream.write_all(&encoded).expect("request writes");
    stream.flush().expect("flush");
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .expect("response reads");
    serde_json::from_str(&line).expect("response parses")
}

#[test]
fn external_simulator_process_recovers_after_restart() {
    let address: SocketAddr = {
        // Bind to an ephemeral port, then release it for the child to reuse.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
        listener.local_addr().expect("addr")
    };
    let database = std::env::temp_dir()
        .join(format!(
            "cockpit-restart-proc-{}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .to_string();
    let scenario = scenario_path();

    // First process: create, start, step a few times.
    let mut first = spawn_simulator(&address, &database);
    assert_eq!(
        request(
            &address,
            json!({ "type": "CreateSimulationRun", "path": scenario })
        )
        .get("ok")
        .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        request(&address, json!({ "type": "StartSimulation" }))
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );
    for _ in 0..8 {
        assert_eq!(
            request(&address, json!({ "type": "StepSimulation" }))
                .get("ok")
                .and_then(Value::as_bool),
            Some(true)
        );
    }
    let snapshot = request(&address, json!({ "type": "GetSimulationSnapshot" }));
    let tick_before = snapshot
        .get("result")
        .and_then(|value| value.get("tick"))
        .and_then(Value::as_u64)
        .expect("tick before restart");
    assert!(tick_before > 0);

    // Kill the process (a real restart, not a graceful shutdown).
    first.kill().expect("first simulator killed");
    first.wait().expect("first simulator reaped");
    sleep(Duration::from_millis(100));

    // Second process against the same database recovers via resume.
    let mut second = spawn_simulator(&address, &database);
    let resumed = request(
        &address,
        json!({
            "type": "ResumeSimulation",
            "scenario_path": scenario,
            "run_id": "run-smoke-in-cockpit",
        }),
    );
    assert_eq!(
        resumed.get("ok").and_then(Value::as_bool),
        Some(true),
        "resume after restart: {resumed:?}"
    );
    assert_eq!(
        resumed
            .get("result")
            .and_then(|value| value.get("tick"))
            .and_then(Value::as_u64),
        Some(tick_before),
        "recovered tick matches pre-restart tick"
    );

    second.kill().expect("second simulator killed");
    second.wait().expect("second simulator reaped");
    let _ = std::fs::remove_file(&database);
    let _ = std::fs::remove_dir_all(format!("{database}.payloads"));
}
