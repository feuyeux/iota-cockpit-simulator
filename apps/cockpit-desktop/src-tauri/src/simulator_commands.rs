use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex},
    time::Duration,
};

use cockpit_simulator::ipc::{
    LiveTurnControl, SimulatorHandler,
    proto::{IPC_VERSION, SimulatorCommand, SimulatorEvent, SimulatorRequest, SimulatorResponse},
};
use iota_core::ipc_client::{
    ConnectionState, ReconnectConfig, backoff_delay_ms, next_backoff_delay_ms,
    spawn_sidecar_with_probe, time_jitter_factor,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SLOW_COMMAND_LOG_THRESHOLD: Duration = Duration::from_secs(1);
const SIMULATOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const SIMULATOR_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const SIMULATOR_HEARTBEAT_MAX_MISSES: u8 = 3;
const SIMULATOR_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

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

fn simulator_binary() -> Option<std::ffi::OsString> {
    std::env::var_os("COCKPIT_SIMULATOR_BIN").or_else(bundled_simulator_binary)
}

fn bundled_simulator_binary() -> Option<std::ffi::OsString> {
    if cfg!(debug_assertions) {
        return None;
    }
    let current_exe = std::env::current_exe().ok()?;
    let binary = bundled_simulator_path(&current_exe)?;
    binary.is_file().then(|| binary.into_os_string())
}

fn bundled_simulator_path(current_exe: &Path) -> Option<PathBuf> {
    let executable_dir = current_exe.parent()?;
    let base_dir = if executable_dir.ends_with("deps") {
        executable_dir.parent().unwrap_or(executable_dir)
    } else {
        executable_dir
    };
    let binary_name = if cfg!(windows) {
        "cockpit-simulator.exe"
    } else {
        "cockpit-simulator"
    };
    Some(base_dir.join(binary_name))
}

enum SimulatorTransport {
    Embedded(Box<SimulatorHandler>),
    Process {
        child: Child,
        address: SocketAddr,
        recording_db: PathBuf,
    },
}

impl SimulatorTransport {
    fn stop_process(&mut self) {
        if let Self::Process { child, .. } = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for SimulatorTransport {
    fn drop(&mut self) {
        self.stop_process();
    }
}

#[derive(Clone)]
pub struct SimulatorState {
    transport: Arc<Mutex<SimulatorTransport>>,
    process_address: Arc<Mutex<Option<SocketAddr>>>,
    connection_state: Arc<Mutex<ConnectionState>>,
    live_turn_control: LiveTurnControl,
    token: String,
    sequence: Arc<Mutex<u64>>,
    workspace_root: PathBuf,
}

impl SimulatorState {
    pub fn new(token: impl Into<String>, workspace_root: PathBuf) -> Self {
        let token = token.into();
        let live_turn_control = LiveTurnControl::default();
        Self {
            transport: Arc::new(Mutex::new(SimulatorTransport::Embedded(Box::new(
                SimulatorHandler::with_live_turn_control(token.clone(), live_turn_control.clone()),
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
        let Some(binary) = simulator_binary() else {
            self.set_connection_state(ConnectionState::Connected);
            return Ok("embedded".to_string());
        };
        let address = simulator_address()?;
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| "simulator transport lock poisoned".to_string())?;

        if let SimulatorTransport::Process {
            address: current_address,
            ..
        } = &*transport
            && simulator_ping(
                *current_address,
                &self.token,
                0,
                Duration::from_millis(200),
                SIMULATOR_HEARTBEAT_TIMEOUT,
            )
        {
            *self
                .process_address
                .lock()
                .map_err(|_| "simulator process address lock poisoned".to_string())? =
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
            .map_err(|_| "simulator process address lock poisoned".to_string())? = None;

        *transport = Self::spawn_process(binary, address, &self.token)?;
        *self
            .process_address
            .lock()
            .map_err(|_| "simulator process address lock poisoned".to_string())? = Some(address);
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

    /// Reconnect to (or respawn) the simulator sidecar using exponential
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
        simulator_ping(
            address,
            &self.token,
            seq,
            Duration::from_secs(1),
            SIMULATOR_HEARTBEAT_TIMEOUT,
        )
    }

    /// Run a heartbeat loop against the simulator sidecar, triggering a
    /// backoff reconnect after `SIMULATOR_HEARTBEAT_MAX_MISSES` consecutive
    /// missed pongs. Intended to run on a dedicated background thread for
    /// the lifetime of the app; returns only if the thread is torn down.
    pub fn run_heartbeat_loop(&self) {
        let mut missed = 0u8;
        let mut seq = 0u64;
        loop {
            std::thread::sleep(SIMULATOR_HEARTBEAT_INTERVAL);
            if self.connection_state() != ConnectionState::Connected {
                continue;
            }
            seq += 1;
            if self.send_heartbeat_ping(seq) {
                missed = 0;
                continue;
            }
            missed += 1;
            if missed >= SIMULATOR_HEARTBEAT_MAX_MISSES {
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
    ) -> Result<SimulatorTransport, String> {
        // Persist committed ticks so the external simulator process can recover its
        // snapshot and event cursor if it is restarted (see the simulator crate's
        // process_restart_recovery integration test).
        let recording_db = std::env::temp_dir().join(format!(
            "cockpit-simulator-recording-{}.sqlite",
            std::process::id()
        ));
        let recording_db_text = recording_db.to_string_lossy().to_string();
        let address_text = address.to_string();
        let child = spawn_sidecar_with_probe(
            binary.as_os_str(),
            SIMULATOR_CONNECT_TIMEOUT,
            Duration::from_millis(20),
            |command| {
                command.args([
                    "serve",
                    "--bind",
                    &address_text,
                    "--session-token",
                    token,
                    "--recording-db",
                    &recording_db_text,
                ]);
            },
            || {
                simulator_ping(
                    address,
                    token,
                    0,
                    Duration::from_millis(20),
                    Duration::from_millis(100),
                )
            },
        )
        .map_err(|error| format!("failed to start cockpit-simulator: {error}"))?;
        Ok(SimulatorTransport::Process {
            child,
            address,
            recording_db,
        })
    }

    pub fn recording_snapshot(&self, run_id: &str) -> Result<cockpit_recording::Recording, String> {
        let recording_db = {
            let transport = self
                .transport
                .lock()
                .map_err(|_| "simulator transport lock poisoned".to_string())?;
            match &*transport {
                SimulatorTransport::Embedded(handler) => {
                    return handler
                        .recording_snapshot(run_id)
                        .ok_or_else(|| format!("recording '{run_id}' is not available"));
                }
                SimulatorTransport::Process { recording_db, .. } => recording_db.clone(),
            }
        };
        let store = cockpit_recording::RecordingStore::open_read_only(
            recording_db
                .to_str()
                .ok_or_else(|| "recording DB path is not UTF-8".to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let recording = store.load(run_id).map_err(|error| error.to_string())?;
        let snapshot = self.dispatch(SimulatorCommand::GetSimulationSnapshot)?;
        validate_durable_recording(&recording, &snapshot)?;
        Ok(recording)
    }

    fn dispatch(&self, command: SimulatorCommand) -> Result<Value, String> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = SimulatorRequest {
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
                .map_err(|_| "simulator transport lock poisoned".to_string())?;
            match &mut *transport {
                SimulatorTransport::Embedded(handler) => {
                    return response_value(handler.dispatch(request));
                }
                SimulatorTransport::Process { address, .. } => *address,
            }
        };

        match request_process(address, &request) {
            Ok(response) => response_value(response),
            Err(error) => {
                self.set_connection_state(ConnectionState::Disconnected);
                if let Err(reconnect_error) = self.connect() {
                    return Err(format!(
                        "{error}; simulator recovery also failed: {reconnect_error}"
                    ));
                }
                // Do not automatically replay the command: the simulator may have
                // committed it before the response was lost.
                Err(format!(
                    "{error}; simulator recovered, command outcome is unknown"
                ))
            }
        }
    }

    fn dispatch_live_blocking(&self, command: SimulatorCommand) -> Result<Value, String> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = SimulatorRequest {
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
                .map_err(|_| "simulator transport lock poisoned".to_string())?;
            match &mut *transport {
                SimulatorTransport::Embedded(handler) => {
                    return response_value(tauri::async_runtime::block_on(
                        handler.dispatch_async(request),
                    ));
                }
                SimulatorTransport::Process { address, .. } => *address,
            }
        };

        match request_process(address, &request) {
            Ok(response) => response_value(response),
            Err(error) => {
                self.set_connection_state(ConnectionState::Disconnected);
                if let Err(reconnect_error) = self.connect() {
                    return Err(format!(
                        "{error}; simulator recovery also failed: {reconnect_error}"
                    ));
                }
                Err(format!(
                    "{error}; simulator recovered, live-step outcome is unknown"
                ))
            }
        }
    }

    fn cancel_live_turn(&self) -> Result<(), String> {
        let address = *self
            .process_address
            .lock()
            .map_err(|_| "simulator process address lock poisoned".to_string())?;
        let Some(address) = address else {
            self.live_turn_control.cancel();
            return Ok(());
        };
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| "sequence lock poisoned".to_string())?;
        *sequence += 1;
        let request = SimulatorRequest {
            version: IPC_VERSION,
            session_token: self.token.clone(),
            correlation_id: format!("desktop-{}", *sequence),
            command: SimulatorCommand::CancelLiveTurn,
        };
        drop(sequence);
        response_value(request_process(address, &request)?).map(|_| ())
    }
}

fn validate_durable_recording(
    recording: &cockpit_recording::Recording,
    snapshot: &Value,
) -> Result<(), String> {
    let durable_tick = recording
        .ticks
        .last()
        .map(|tick| tick.tick.saturating_add(1))
        .unwrap_or(0);
    validate_durable_position(&recording.run_id, durable_tick, snapshot)
}

fn validate_durable_position(
    recording_run_id: &str,
    durable_tick: u64,
    snapshot: &Value,
) -> Result<(), String> {
    let snapshot_run_id = snapshot
        .get("runId")
        .and_then(Value::as_str)
        .ok_or_else(|| "simulator snapshot is missing runId".to_string())?;
    let snapshot_tick = snapshot
        .get("tick")
        .and_then(Value::as_u64)
        .ok_or_else(|| "simulator snapshot is missing tick".to_string())?;
    if snapshot_run_id != recording_run_id || snapshot_tick != durable_tick {
        return Err(format!(
            "recording is not durable at the current simulator snapshot: run {} tick {}, durable run {} tick {}",
            snapshot_run_id, snapshot_tick, recording_run_id, durable_tick
        ));
    }
    Ok(())
}

fn simulator_address() -> Result<SocketAddr, String> {
    "127.0.0.1:47701"
        .parse()
        .map_err(|error| format!("invalid simulator address: {error}"))
}

fn simulator_ping(
    address: SocketAddr,
    token: &str,
    seq: u64,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> bool {
    let correlation_id = format!("heartbeat-{seq}");
    let request = SimulatorRequest {
        version: IPC_VERSION,
        session_token: token.to_string(),
        correlation_id: correlation_id.clone(),
        command: SimulatorCommand::Ping { seq },
    };
    request_process_with_timeouts(address, &request, connect_timeout, read_timeout)
        .is_ok_and(|response| heartbeat_response_matches(&response, seq, &correlation_id))
}

fn heartbeat_response_matches(
    response: &SimulatorResponse,
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

fn request_process(
    address: SocketAddr,
    request: &SimulatorRequest,
) -> Result<SimulatorResponse, String> {
    let read_timeout = live_command_read_timeout(&request.command);
    request_process_with_timeouts(address, request, Duration::from_millis(1_000), read_timeout)
}

/// Read-side margin added on top of a live command's own backend budget to
/// cover the ACP child-process spawn and IPC round trip surrounding the timed
/// backend work.
const LIVE_COMMAND_IPC_MARGIN: Duration = Duration::from_secs(30);

/// Decide how long the desktop waits for a simulator response, per command.
///
/// The read timeout must outlast the simulator's own work for that command, or the
/// desktop severs the TCP connection while the simulator is still legitimately
/// busy — which surfaces as a spurious "simulator disconnected" and even triggers a
/// reconnect that respawns the sidecar mid-operation.
///
/// Most commands are cheap synchronous handler calls, so a short fixed budget is
/// right. The live commands are the exception: they drive a real backend (Hermes
/// via iota-core ACP) turn or cold-start warm-up whose deadline is the
/// caller-supplied `timeout_ms` (up to 120s, and 60s by default because Hermes
/// initializes its ACP tool surface before the first prompt). `StepLiveSimulation`
/// carries no explicit budget in the command — its per-turn deadline was fixed
/// when the run was created and a single step may span several tool-call rounds —
/// so it keeps a generous fixed ceiling.
fn live_command_read_timeout(command: &SimulatorCommand) -> Duration {
    match command {
        SimulatorCommand::StepLiveSimulation => Duration::from_secs(600),
        SimulatorCommand::CreateLiveSimulationRun { timeout_ms, .. }
        | SimulatorCommand::ResumeLiveSimulation { timeout_ms, .. } => {
            Duration::from_millis(*timeout_ms).saturating_add(LIVE_COMMAND_IPC_MARGIN)
        }
        _ => Duration::from_millis(5_000),
    }
}

fn request_process_with_timeouts(
    address: SocketAddr,
    request: &SimulatorRequest,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<SimulatorResponse, String> {
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)
        .map_err(|error| format!("simulator disconnected: {error}"))?;
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
        .map_err(|error| format!("simulator response failed: {error}"))?;
    if bytes_read == 0 {
        return Err("simulator response was empty".to_string());
    }
    serde_json::from_str(&line).map_err(|error| format!("simulator response invalid: {error}"))
}

fn response_value(response: SimulatorResponse) -> Result<Value, String> {
    if response.ok {
        Ok(response.result.unwrap_or(Value::Null))
    } else {
        Err(response
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "simulator command failed".to_string()))
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
pub fn connect_simulator(state: tauri::State<'_, SimulatorState>) -> Result<String, String> {
    state.connect()
}

#[tauri::command]
pub fn validate_scenario(
    state: tauri::State<'_, SimulatorState>,
    path: String,
) -> Result<ScenarioSummary, String> {
    let resolved_path = state.resolve_path(&path)?;
    serde_json::from_value(state.dispatch(SimulatorCommand::ValidateScenario {
        path: resolved_path,
    })?)
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_live_simulation_run(
    state: tauri::State<'_, SimulatorState>,
    path: String,
    timeout_ms: u64,
) -> Result<LiveRunSummary, String> {
    let resolved_path = state.resolve_path(&path)?;
    let started = std::time::Instant::now();
    let owned = state.inner().clone();
    let result: Result<LiveRunSummary, String> = tauri::async_runtime::spawn_blocking(move || {
        owned.dispatch_live_blocking(SimulatorCommand::CreateLiveSimulationRun {
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
            "simulator command slow: CreateLiveSimulationRun run_id={} backend={} elapsed_ms={}",
            run.run_id,
            run.backend,
            elapsed.as_millis()
        ),
        Ok(_) => {}
        Err(error) => eprintln!(
            "simulator command failed: CreateLiveSimulationRun elapsed_ms={} error={error}",
            elapsed.as_millis()
        ),
    }
    result
}

#[tauri::command]
pub fn start_simulation(state: tauri::State<'_, SimulatorState>) -> Result<(), String> {
    state
        .dispatch(SimulatorCommand::StartSimulation)
        .map(|_| ())
}

#[tauri::command]
pub fn pause_simulation(state: tauri::State<'_, SimulatorState>) -> Result<(), String> {
    state
        .dispatch(SimulatorCommand::PauseSimulation)
        .map(|_| ())
}

#[tauri::command]
pub async fn step_live_simulation(
    state: tauri::State<'_, SimulatorState>,
) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let owned = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        owned.dispatch_live_blocking(SimulatorCommand::StepLiveSimulation)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let elapsed = started.elapsed();
    match &result {
        Ok(step) if should_log_slow_operation(elapsed) => eprintln!(
            "simulator command slow: StepLiveSimulation tick={} elapsed_ms={}",
            step.get("tick")
                .and_then(Value::as_u64)
                .map_or_else(|| "unknown".to_string(), |tick| tick.to_string()),
            elapsed.as_millis()
        ),
        Ok(_) => {}
        Err(error) => eprintln!(
            "simulator command failed: StepLiveSimulation elapsed_ms={} error={error}",
            elapsed.as_millis()
        ),
    }
    result
}

#[tauri::command]
pub fn stop_simulation(state: tauri::State<'_, SimulatorState>) -> Result<(), String> {
    state.dispatch(SimulatorCommand::StopSimulation).map(|_| ())
}

#[tauri::command]
pub async fn resume_live_simulation(
    state: tauri::State<'_, SimulatorState>,
    scenario_path: String,
    run_id: String,
    timeout_ms: u64,
) -> Result<LiveRunSummary, String> {
    let resolved_scenario_path = state.resolve_path(&scenario_path)?;
    let owned = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        owned.dispatch_live_blocking(SimulatorCommand::ResumeLiveSimulation {
            scenario_path: resolved_scenario_path,
            run_id,
            timeout_ms,
        })
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result)
    .and_then(|result| serde_json::from_value(result).map_err(|error| error.to_string()))
}

#[tauri::command]
pub fn resume_simulation(
    state: tauri::State<'_, SimulatorState>,
    scenario_path: String,
    run_id: String,
) -> Result<(), String> {
    let resolved_scenario_path = state.resolve_path(&scenario_path)?;
    state
        .dispatch(SimulatorCommand::ResumeSimulation {
            scenario_path: resolved_scenario_path,
            run_id,
        })
        .map(|_| ())
}

#[tauri::command]
pub fn approve_action(
    state: tauri::State<'_, SimulatorState>,
    request_id: String,
) -> Result<Value, String> {
    state.dispatch(SimulatorCommand::ApproveAction { request_id })
}

#[tauri::command]
pub fn reject_action(
    state: tauri::State<'_, SimulatorState>,
    request_id: String,
    reason: Option<String>,
) -> Result<Value, String> {
    state.dispatch(SimulatorCommand::RejectAction { request_id, reason })
}

#[tauri::command]
pub fn cancel_agent_turn(state: tauri::State<'_, SimulatorState>) -> Result<(), String> {
    state
        .dispatch(SimulatorCommand::CancelAgentTurn)
        .map(|_| ())
}

#[tauri::command]
pub fn cancel_live_turn(state: tauri::State<'_, SimulatorState>) -> Result<(), String> {
    state.cancel_live_turn()
}

#[tauri::command]
pub fn set_approval_required(
    state: tauri::State<'_, SimulatorState>,
    required: bool,
) -> Result<(), String> {
    state
        .dispatch(SimulatorCommand::SetApprovalRequired { required })
        .map(|_| ())
}

#[tauri::command]
pub fn start_replay(
    state: tauri::State<'_, SimulatorState>,
    scenario_path: String,
    recording_path: String,
) -> Result<Value, String> {
    let resolved_scenario_path = state.resolve_path(&scenario_path)?;
    let resolved_recording_path = state.resolve_path(&recording_path)?;
    state.dispatch(SimulatorCommand::StartReplay {
        scenario_path: resolved_scenario_path,
        recording_path: resolved_recording_path,
    })
}

#[tauri::command]
pub fn diff_recordings(
    state: tauri::State<'_, SimulatorState>,
    source_recording_path: String,
    candidate_recording_path: String,
) -> Result<Value, String> {
    let resolved_source_path = state.resolve_path(&source_recording_path)?;
    let resolved_candidate_path = state.resolve_path(&candidate_recording_path)?;
    state.dispatch(SimulatorCommand::DiffRecordings {
        source_recording_path: resolved_source_path,
        candidate_recording_path: resolved_candidate_path,
    })
}

#[tauri::command]
pub fn get_simulation_events(
    state: tauri::State<'_, SimulatorState>,
    cursor: Option<u64>,
) -> Result<SimulationEventBatch, String> {
    let result = state.dispatch(SimulatorCommand::GetSimulationEvents { cursor })?;
    serde_json::from_value(result).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationEventBatch {
    pub events: Vec<SimulatorEvent>,
    pub next_cursor: u64,
    pub first_available_cursor: u64,
    pub reset_required: bool,
}

#[tauri::command]
pub fn get_simulation_snapshot(state: tauri::State<'_, SimulatorState>) -> Result<Value, String> {
    state.dispatch(SimulatorCommand::GetSimulationSnapshot)
}

#[cfg(test)]
#[path = "simulator_commands_tests.rs"]
mod tests;
