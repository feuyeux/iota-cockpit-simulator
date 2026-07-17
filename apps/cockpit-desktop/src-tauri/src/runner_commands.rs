use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex},
    time::Duration,
};

use cockpit_runner::ipc::{
    LiveTurnControl, RunnerHandler,
    proto::{IPC_VERSION, RunnerCommand, RunnerEvent, RunnerRequest, RunnerResponse},
};
use iota_core::ipc_client::{
    ConnectionState, ReconnectConfig, backoff_delay_ms, next_backoff_delay_ms,
    spawn_sidecar_with_probe, time_jitter_factor,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SLOW_COMMAND_LOG_THRESHOLD: Duration = Duration::from_secs(1);
const RUNNER_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const RUNNER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const RUNNER_HEARTBEAT_MAX_MISSES: u8 = 3;
const RUNNER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

fn should_log_slow_operation(elapsed: Duration) -> bool {
    elapsed >= SLOW_COMMAND_LOG_THRESHOLD
}

/// Lexically normalize a path by collapsing `.` and `..` segments, without
/// touching the filesystem. This must not use `fs::canonicalize`: the
/// scenario path may not exist yet at resolution time (validation happens
/// afterward), and canonicalize requires the path to exist.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn runner_binary() -> Option<std::ffi::OsString> {
    std::env::var_os("COCKPIT_RUNNER_BIN").or_else(bundled_runner_binary)
}

fn bundled_runner_binary() -> Option<std::ffi::OsString> {
    if cfg!(debug_assertions) {
        return None;
    }
    let current_exe = std::env::current_exe().ok()?;
    let binary = bundled_runner_path(&current_exe)?;
    binary.is_file().then(|| binary.into_os_string())
}

fn bundled_runner_path(current_exe: &Path) -> Option<PathBuf> {
    let executable_dir = current_exe.parent()?;
    let base_dir = if executable_dir.ends_with("deps") {
        executable_dir.parent().unwrap_or(executable_dir)
    } else {
        executable_dir
    };
    let binary_name = if cfg!(windows) {
        "cockpit-runner.exe"
    } else {
        "cockpit-runner"
    };
    Some(base_dir.join(binary_name))
}

enum RunnerTransport {
    Embedded(Box<RunnerHandler>),
    Process { child: Child, address: SocketAddr },
}

impl RunnerTransport {
    fn stop_process(&mut self) {
        if let Self::Process { child, .. } = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for RunnerTransport {
    fn drop(&mut self) {
        self.stop_process();
    }
}

#[derive(Clone)]
pub struct RunnerState {
    transport: Arc<Mutex<RunnerTransport>>,
    process_address: Arc<Mutex<Option<SocketAddr>>>,
    connection_state: Arc<Mutex<ConnectionState>>,
    live_turn_control: LiveTurnControl,
    token: String,
    sequence: Arc<Mutex<u64>>,
    workspace_root: PathBuf,
}

impl RunnerState {
    pub fn new(token: impl Into<String>, workspace_root: PathBuf) -> Self {
        let token = token.into();
        let live_turn_control = LiveTurnControl::default();
        Self {
            transport: Arc::new(Mutex::new(RunnerTransport::Embedded(Box::new(
                RunnerHandler::with_live_turn_control(token.clone(), live_turn_control.clone()),
            )))),
            process_address: Arc::new(Mutex::new(None)),
            connection_state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            live_turn_control,
            token,
            sequence: Arc::new(Mutex::new(0)),
            workspace_root,
        }
    }

    /// Resolve a path relative to the workspace root if it's not already absolute.
    ///
    /// This desktop command mirrors a native "open file" dialog: the caller
    /// is the machine's own user, so reading an arbitrary path they can
    /// already reach with a file manager is not a privilege escalation.
    /// The one input class this must still reject outright is a path
    /// containing an embedded NUL byte, which is invalid on every platform
    /// and would otherwise reach the OS path APIs as malformed input. For a
    /// *relative* path, resolve it against the workspace root and reject
    /// the result if lexical normalization (`..` segments) would walk the
    /// path back out of the workspace root - a defense-in-depth guard
    /// against a stray `..` segment silently escaping the workspace when
    /// the caller only meant to reference a bundled scenario file.
    /// Absolute paths bypass this check by design, matching a native file
    /// picker that can point anywhere on disk.
    fn resolve_path(&self, path: &str) -> Result<String, String> {
        if path.contains('\0') {
            return Err("scenario path must not contain a NUL byte".to_string());
        }
        let path_buf = Path::new(path);
        if path_buf.is_absolute() {
            return Ok(path.to_string());
        }
        let joined = self.workspace_root.join(path_buf);
        let normalized = normalize_lexically(&joined);
        if !normalized.starts_with(&self.workspace_root) {
            return Err(format!(
                "scenario path '{path}' resolves outside the workspace root"
            ));
        }
        Ok(joined.to_string_lossy().to_string())
    }

    pub fn connect(&self) -> Result<String, String> {
        let Some(binary) = runner_binary() else {
            self.set_connection_state(ConnectionState::Connected);
            return Ok("embedded".to_string());
        };
        let address = runner_address()?;
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| "runner transport lock poisoned".to_string())?;

        if let RunnerTransport::Process {
            address: current_address,
            ..
        } = &*transport
            && runner_ping(
                *current_address,
                &self.token,
                0,
                Duration::from_millis(200),
                RUNNER_HEARTBEAT_TIMEOUT,
            )
        {
            *self
                .process_address
                .lock()
                .map_err(|_| "runner process address lock poisoned".to_string())? =
                Some(*current_address);
            self.set_connection_state(ConnectionState::Connected);
            return Ok("process".to_string());
        }

        // The existing child must release the fixed loopback port before a
        // replacement is spawned. Otherwise a raw readiness probe can connect
        // to the old process and incorrectly bless a new child that failed to
        // bind.
        transport.stop_process();
        *self
            .process_address
            .lock()
            .map_err(|_| "runner process address lock poisoned".to_string())? = None;

        *transport = Self::spawn_process(binary, address, &self.token)?;
        *self
            .process_address
            .lock()
            .map_err(|_| "runner process address lock poisoned".to_string())? = Some(address);
        self.set_connection_state(ConnectionState::Connected);
        Ok("process".to_string())
    }

    fn set_connection_state(&self, state: ConnectionState) {
        if let Ok(mut guard) = self.connection_state.lock() {
            *guard = state;
        }
    }

    pub fn connection_state(&self) -> ConnectionState {
        self.connection_state
            .lock()
            .map(|guard| *guard)
            .unwrap_or(ConnectionState::Disconnected)
    }

    /// Reconnect to (or respawn) the runner sidecar using exponential
    /// backoff with jitter, shared with iota-desktop's daemon client via
    /// `iota_core::ipc_client`. Blocking: intended to run on a background
    /// thread, not the async/UI-facing command path.
    fn reconnect_with_backoff(&self) -> Result<(), String> {
        self.set_connection_state(ConnectionState::Reconnecting);
        let config = ReconnectConfig::default();
        let mut current_delay_ms = config.initial_delay_ms;
        loop {
            let actual_delay = backoff_delay_ms(current_delay_ms, &config, time_jitter_factor());
            current_delay_ms = next_backoff_delay_ms(current_delay_ms, &config);
            std::thread::sleep(Duration::from_millis(actual_delay));

            if self.connect().is_ok() {
                return Ok(());
            }
        }
    }

    /// Send a lightweight authenticated Ping and require a matching Pong.
    fn send_heartbeat_ping(&self, seq: u64) -> bool {
        let address = match self.process_address.lock() {
            Ok(guard) => *guard,
            Err(_) => return false,
        };
        let Some(address) = address else {
            // Embedded transport: always considered alive.
            return true;
        };
        runner_ping(
            address,
            &self.token,
            seq,
            Duration::from_secs(1),
            RUNNER_HEARTBEAT_TIMEOUT,
        )
    }

    /// Run a heartbeat loop against the runner sidecar, triggering a
    /// backoff reconnect after `RUNNER_HEARTBEAT_MAX_MISSES` consecutive
    /// missed pongs. Intended to run on a dedicated background thread for
    /// the lifetime of the app; returns only if the thread is torn down.
    pub fn run_heartbeat_loop(&self) {
        let mut missed = 0u8;
        let mut seq = 0u64;
        loop {
            std::thread::sleep(RUNNER_HEARTBEAT_INTERVAL);
            if self.connection_state() != ConnectionState::Connected {
                continue;
            }
            seq += 1;
            if self.send_heartbeat_ping(seq) {
                missed = 0;
                continue;
            }
            missed += 1;
            if missed >= RUNNER_HEARTBEAT_MAX_MISSES {
                missed = 0;
                self.set_connection_state(ConnectionState::Disconnected);
                let _ = self.reconnect_with_backoff();
            }
        }
    }

    fn spawn_process(
        binary: std::ffi::OsString,
        address: SocketAddr,
        token: &str,
    ) -> Result<RunnerTransport, String> {
        // Persist committed ticks so the external runner process can recover its
        // snapshot and event cursor if it is restarted (see the runner crate's
        // process_restart_recovery integration test).
        let recording_db = std::env::temp_dir()
            .join("cockpit-runner-recording.sqlite")
            .to_string_lossy()
            .to_string();
        let address_text = address.to_string();
        let child = spawn_sidecar_with_probe(
            binary.as_os_str(),
            RUNNER_CONNECT_TIMEOUT,
            Duration::from_millis(20),
            |command| {
                command.args([
                    "serve",
                    "--bind",
                    &address_text,
                    "--session-token",
                    token,
                    "--recording-db",
                    &recording_db,
                ]);
            },
            || {
                runner_ping(
                    address,
                    token,
                    0,
                    Duration::from_millis(20),
                    Duration::from_millis(100),
                )
            },
        )
        .map_err(|error| format!("failed to start cockpit-runner: {error}"))?;
        Ok(RunnerTransport::Process { child, address })
    }

    fn dispatch(&self, command: RunnerCommand) -> Result<Value, String> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = RunnerRequest {
            version: IPC_VERSION,
            session_token: self.token.clone(),
            correlation_id: format!("desktop-{}", *sequence),
            command,
        };
        drop(sequence);
        let address = {
            let mut transport = self
                .transport
                .lock()
                .map_err(|_| "runner transport lock poisoned".to_string())?;
            match &mut *transport {
                RunnerTransport::Embedded(handler) => {
                    return response_value(handler.dispatch(request));
                }
                RunnerTransport::Process { address, .. } => *address,
            }
        };

        match request_process(address, &request) {
            Ok(response) => response_value(response),
            Err(error) => {
                self.set_connection_state(ConnectionState::Disconnected);
                if let Err(reconnect_error) = self.connect() {
                    return Err(format!(
                        "{error}; runner recovery also failed: {reconnect_error}"
                    ));
                }
                // Do not automatically replay the command: the runner may have
                // committed it before the response was lost.
                Err(format!(
                    "{error}; runner recovered, command outcome is unknown"
                ))
            }
        }
    }

    fn dispatch_live_blocking(&self, command: RunnerCommand) -> Result<Value, String> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = RunnerRequest {
            version: IPC_VERSION,
            session_token: self.token.clone(),
            correlation_id: format!("desktop-{}", *sequence),
            command,
        };
        drop(sequence);

        let address = {
            let mut transport = self
                .transport
                .lock()
                .map_err(|_| "runner transport lock poisoned".to_string())?;
            match &mut *transport {
                RunnerTransport::Embedded(handler) => {
                    return response_value(tauri::async_runtime::block_on(
                        handler.dispatch_async(request),
                    ));
                }
                RunnerTransport::Process { address, .. } => *address,
            }
        };

        match request_process(address, &request) {
            Ok(response) => response_value(response),
            Err(error) => {
                self.set_connection_state(ConnectionState::Disconnected);
                if let Err(reconnect_error) = self.connect() {
                    return Err(format!(
                        "{error}; runner recovery also failed: {reconnect_error}"
                    ));
                }
                Err(format!(
                    "{error}; runner recovered, live-step outcome is unknown"
                ))
            }
        }
    }

    fn cancel_live_turn(&self) -> Result<(), String> {
        let address = *self
            .process_address
            .lock()
            .map_err(|_| "runner process address lock poisoned".to_string())?;
        let Some(address) = address else {
            self.live_turn_control.cancel();
            return Ok(());
        };
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = RunnerRequest {
            version: IPC_VERSION,
            session_token: self.token.clone(),
            correlation_id: format!("desktop-{}", *sequence),
            command: RunnerCommand::CancelLiveTurn,
        };
        drop(sequence);
        response_value(request_process(address, &request)?).map(|_| ())
    }
}

fn runner_address() -> Result<SocketAddr, String> {
    "127.0.0.1:47701"
        .parse()
        .map_err(|error| format!("invalid runner address: {error}"))
}

fn runner_ping(
    address: SocketAddr,
    token: &str,
    seq: u64,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> bool {
    let correlation_id = format!("heartbeat-{seq}");
    let request = RunnerRequest {
        version: IPC_VERSION,
        session_token: token.to_string(),
        correlation_id: correlation_id.clone(),
        command: RunnerCommand::Ping { seq },
    };
    request_process_with_timeouts(address, &request, connect_timeout, read_timeout)
        .is_ok_and(|response| heartbeat_response_matches(&response, seq, &correlation_id))
}

fn heartbeat_response_matches(
    response: &RunnerResponse,
    expected_seq: u64,
    expected_correlation_id: &str,
) -> bool {
    response.version == IPC_VERSION
        && response.ok
        && response.correlation_id == expected_correlation_id
        && response.error.is_none()
        && response.result.as_ref().is_some_and(|result| {
            result.get("pong").and_then(Value::as_bool) == Some(true)
                && result.get("seq").and_then(Value::as_u64) == Some(expected_seq)
        })
}

fn request_process(address: SocketAddr, request: &RunnerRequest) -> Result<RunnerResponse, String> {
    let read_timeout = if matches!(request.command, RunnerCommand::StepLiveSimulation) {
        Duration::from_secs(600)
    } else {
        Duration::from_millis(5_000)
    };
    request_process_with_timeouts(address, request, Duration::from_millis(1_000), read_timeout)
}

fn request_process_with_timeouts(
    address: SocketAddr,
    request: &RunnerRequest,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<RunnerResponse, String> {
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)
        .map_err(|error| format!("runner disconnected: {error}"))?;
    stream
        .set_read_timeout(Some(read_timeout))
        .map_err(|error| error.to_string())?;
    let mut encoded = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    stream
        .write_all(&encoded)
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut line = String::new();
    let bytes_read = BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| format!("runner response failed: {error}"))?;
    if bytes_read == 0 {
        return Err("runner response was empty".to_string());
    }
    serde_json::from_str(&line).map_err(|error| format!("runner response invalid: {error}"))
}

fn response_value(response: RunnerResponse) -> Result<Value, String> {
    if response.ok {
        Ok(response.result.unwrap_or(Value::Null))
    } else {
        Err(response
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "runner command failed".to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioSummary {
    pub id: String,
    pub path: String,
    pub schema_version: u32,
    pub scenario_hash: String,
    pub seed: u64,
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRunSummary {
    pub run_id: String,
    pub backend: String,
}

#[tauri::command]
pub fn connect_runner(state: tauri::State<'_, RunnerState>) -> Result<String, String> {
    state.connect()
}

#[tauri::command]
pub fn validate_scenario(
    state: tauri::State<'_, RunnerState>,
    path: String,
) -> Result<ScenarioSummary, String> {
    let resolved_path = state.resolve_path(&path)?;
    serde_json::from_value(state.dispatch(RunnerCommand::ValidateScenario {
        path: resolved_path,
    })?)
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_live_simulation_run(
    state: tauri::State<'_, RunnerState>,
    path: String,
    timeout_ms: u64,
) -> Result<LiveRunSummary, String> {
    let resolved_path = state.resolve_path(&path)?;
    let started = std::time::Instant::now();
    let owned = state.inner().clone();
    let result: Result<LiveRunSummary, String> = tauri::async_runtime::spawn_blocking(move || {
        owned.dispatch_live_blocking(RunnerCommand::CreateLiveSimulationRun {
            path: resolved_path,
            timeout_ms,
        })
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result)
    .and_then(|result| serde_json::from_value(result).map_err(|error| error.to_string()));
    let elapsed = started.elapsed();
    match &result {
        Ok(run) if should_log_slow_operation(elapsed) => eprintln!(
            "runner command slow: CreateLiveSimulationRun run_id={} backend={} elapsed_ms={}",
            run.run_id,
            run.backend,
            elapsed.as_millis()
        ),
        Ok(_) => {}
        Err(error) => eprintln!(
            "runner command failed: CreateLiveSimulationRun elapsed_ms={} error={error}",
            elapsed.as_millis()
        ),
    }
    result
}

#[tauri::command]
pub fn start_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::StartSimulation).map(|_| ())
}

#[tauri::command]
pub fn pause_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::PauseSimulation).map(|_| ())
}

#[tauri::command]
pub async fn step_live_simulation(state: tauri::State<'_, RunnerState>) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let owned = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        owned.dispatch_live_blocking(RunnerCommand::StepLiveSimulation)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let elapsed = started.elapsed();
    match &result {
        Ok(step) if should_log_slow_operation(elapsed) => eprintln!(
            "runner command slow: StepLiveSimulation tick={} elapsed_ms={}",
            step.get("tick")
                .and_then(Value::as_u64)
                .map_or_else(|| "unknown".to_string(), |tick| tick.to_string()),
            elapsed.as_millis()
        ),
        Ok(_) => {}
        Err(error) => eprintln!(
            "runner command failed: StepLiveSimulation elapsed_ms={} error={error}",
            elapsed.as_millis()
        ),
    }
    result
}

#[tauri::command]
pub fn stop_simulation(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::StopSimulation).map(|_| ())
}

#[tauri::command]
pub fn resume_simulation(
    state: tauri::State<'_, RunnerState>,
    scenario_path: String,
    run_id: String,
) -> Result<(), String> {
    let resolved_scenario_path = state.resolve_path(&scenario_path)?;
    state
        .dispatch(RunnerCommand::ResumeSimulation {
            scenario_path: resolved_scenario_path,
            run_id,
        })
        .map(|_| ())
}

#[tauri::command]
pub fn approve_action(
    state: tauri::State<'_, RunnerState>,
    request_id: String,
) -> Result<Value, String> {
    state.dispatch(RunnerCommand::ApproveAction { request_id })
}

#[tauri::command]
pub fn reject_action(
    state: tauri::State<'_, RunnerState>,
    request_id: String,
    reason: Option<String>,
) -> Result<Value, String> {
    state.dispatch(RunnerCommand::RejectAction { request_id, reason })
}

#[tauri::command]
pub fn cancel_agent_turn(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.dispatch(RunnerCommand::CancelAgentTurn).map(|_| ())
}

#[tauri::command]
pub fn cancel_live_turn(state: tauri::State<'_, RunnerState>) -> Result<(), String> {
    state.cancel_live_turn()
}

#[tauri::command]
pub fn set_approval_required(
    state: tauri::State<'_, RunnerState>,
    required: bool,
) -> Result<(), String> {
    state
        .dispatch(RunnerCommand::SetApprovalRequired { required })
        .map(|_| ())
}

#[tauri::command]
pub fn start_replay(
    state: tauri::State<'_, RunnerState>,
    scenario_path: String,
    recording_path: String,
) -> Result<Value, String> {
    let resolved_scenario_path = state.resolve_path(&scenario_path)?;
    let resolved_recording_path = state.resolve_path(&recording_path)?;
    state.dispatch(RunnerCommand::StartReplay {
        scenario_path: resolved_scenario_path,
        recording_path: resolved_recording_path,
    })
}

#[tauri::command]
pub fn diff_recordings(
    state: tauri::State<'_, RunnerState>,
    source_recording_path: String,
    candidate_recording_path: String,
) -> Result<Value, String> {
    let resolved_source_path = state.resolve_path(&source_recording_path)?;
    let resolved_candidate_path = state.resolve_path(&candidate_recording_path)?;
    state.dispatch(RunnerCommand::DiffRecordings {
        source_recording_path: resolved_source_path,
        candidate_recording_path: resolved_candidate_path,
    })
}

#[tauri::command]
pub fn get_simulation_events(
    state: tauri::State<'_, RunnerState>,
    cursor: Option<u64>,
) -> Result<SimulationEventBatch, String> {
    let result = state.dispatch(RunnerCommand::GetSimulationEvents { cursor })?;
    serde_json::from_value(result).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationEventBatch {
    pub events: Vec<RunnerEvent>,
    pub next_cursor: u64,
    pub first_available_cursor: u64,
    pub reset_required: bool,
}

#[tauri::command]
pub fn get_simulation_snapshot(state: tauri::State<'_, RunnerState>) -> Result<Value, String> {
    state.dispatch(RunnerCommand::GetSimulationSnapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_operation_logging_uses_a_one_second_threshold() {
        assert!(!should_log_slow_operation(
            std::time::Duration::from_millis(999)
        ));
        assert!(should_log_slow_operation(std::time::Duration::from_secs(1)));
    }

    #[test]
    fn bundled_runner_is_resolved_next_to_the_desktop_executable() {
        let executable = Path::new("target/release/cockpit-desktop");
        let expected = Path::new("target/release").join(if cfg!(windows) {
            "cockpit-runner.exe"
        } else {
            "cockpit-runner"
        });

        assert_eq!(bundled_runner_path(executable), Some(expected));
    }

    #[test]
    fn bundled_runner_moves_out_of_the_test_deps_directory() {
        let executable = Path::new("target/debug/deps/cockpit-desktop-test");
        let expected = Path::new("target/debug").join(if cfg!(windows) {
            "cockpit-runner.exe"
        } else {
            "cockpit-runner"
        });

        assert_eq!(bundled_runner_path(executable), Some(expected));
    }

    fn pong_response(seq: u64) -> RunnerResponse {
        RunnerResponse {
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

    fn state_with_workspace_root(root: &str) -> RunnerState {
        RunnerState::new("test-token", PathBuf::from(root))
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
}
