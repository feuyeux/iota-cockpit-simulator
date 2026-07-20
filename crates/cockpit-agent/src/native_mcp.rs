use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use cockpit_world::{
    SimulationScenario, clock::RunStatus, simulation::Simulation, world::WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{LocalMcpServer, ToolDefinition, ToolRequest, ToolResponse, redact_json};

pub const NATIVE_MCP_PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_NATIVE_TOOL_COST_BUDGET: u32 = 16;
/// Eight simulation operations plus an initial and one corrected decision envelope.
pub const DEFAULT_NATIVE_TOOL_CALL_BUDGET: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMcpCall {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    pub response: ToolResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMcpTurnState {
    pub protocol_version: u16,
    pub generation: String,
    pub scenario: SimulationScenario,
    pub status: RunStatus,
    pub snapshot: WorldSnapshot,
    pub server: LocalMcpServer,
    pub human_id: String,
    pub tool_definitions: Vec<ToolDefinition>,
    pub max_calls: usize,
    pub max_cost: u32,
    pub expires_at_unix_ms: u64,
    pub calls: Vec<NativeMcpCall>,
}

#[cfg(any(unix, windows))]
fn private_state_platform_guard() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn private_state_platform_guard() -> Result<(), String> {
    Err(
        "native MCP is disabled: this build cannot guarantee owner-only state-file permissions"
            .to_string(),
    )
}

#[cfg(windows)]
mod windows_private_state {
    use std::{
        ffi::c_void,
        fs::File,
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        path::Path,
        ptr::null_mut,
    };

    type Handle = *mut c_void;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const CREATE_NEW: u32 = 1;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
    const SDDL_REVISION_1: u32 = 1;
    const SE_FILE_OBJECT: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        security_descriptor: *mut c_void,
        inherit_handle: i32,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_security_descriptor: *const u16,
            string_revision: u32,
            security_descriptor: *mut *mut c_void,
            security_descriptor_size: *mut u32,
        ) -> i32;
        fn GetNamedSecurityInfoW(
            object_name: *const u16,
            object_type: u32,
            security_information: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            security_descriptor: *mut *mut c_void,
        ) -> u32;
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
            security_descriptor: *const c_void,
            string_revision: u32,
            security_information: u32,
            string_security_descriptor: *mut *mut u16,
            string_security_descriptor_len: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut SecurityAttributes,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: Handle,
        ) -> Handle;
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `OW` is Windows' well-known Owner Rights SID. A protected DACL with one
    /// full-access ACE for OW grants access to the file owner only and prevents
    /// inheriting broader temp-directory ACLs.
    pub fn create_owner_only(path: &Path) -> Result<File, String> {
        let sddl: Vec<u16> = "D:P(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect();
        let mut descriptor = null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 || descriptor.is_null() {
            return Err(format!(
                "failed to create owner-only Windows security descriptor: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            security_descriptor: descriptor,
            inherit_handle: 0,
        };
        let path_wide = wide(path);
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_WRITE,
                0,
                &mut attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        unsafe {
            LocalFree(descriptor);
        }
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "failed to create owner-only Native MCP state: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    pub fn replace_atomically(source: &Path, destination: &Path) -> Result<(), String> {
        let source = wide(source);
        let destination = wide(destination);
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(format!(
                "failed to atomically replace Native MCP state: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn verify_owner_only(path: &Path) -> Result<(), String> {
        let path = wide(path);
        let mut descriptor = null_mut();
        let status = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 || descriptor.is_null() {
            return Err(format!(
                "failed to read Native MCP state ACL: Windows error {status}"
            ));
        }
        let mut sddl_ptr: *mut u16 = null_mut();
        let mut sddl_len = 0;
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut sddl_ptr,
                &mut sddl_len,
            )
        };
        if converted == 0 || sddl_ptr.is_null() {
            unsafe {
                LocalFree(descriptor);
            }
            return Err(format!(
                "failed to render Native MCP state ACL: {}",
                std::io::Error::last_os_error()
            ));
        }
        let sddl = String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(sddl_ptr, sddl_len as usize)
        });
        unsafe {
            LocalFree(sddl_ptr.cast());
            LocalFree(descriptor);
        }
        if !super::is_owner_only_windows_sddl(&sddl) {
            return Err(format!("Native MCP state ACL is not owner-only: {sddl}"));
        }
        Ok(())
    }
}

fn is_owner_only_windows_sddl(sddl: &str) -> bool {
    let owner_ace = sddl.contains("(A;;FA;;;OW)")
        || sddl.contains("(A;;FA;;;S-1-3-4)")
        || sddl.contains("(A;;GA;;;OW)")
        || sddl.contains("(A;;GA;;;S-1-3-4)");
    sddl.starts_with("D:P") && owner_ace && sddl.matches('(').count() == 1
}
impl NativeMcpTurnState {
    pub fn new(
        generation: String,
        simulation: &Simulation,
        server: &LocalMcpServer,
        human_id: String,
        tool_definitions: Vec<ToolDefinition>,
        lifetime_ms: u64,
    ) -> Self {
        Self {
            protocol_version: NATIVE_MCP_PROTOCOL_VERSION,
            generation,
            scenario: simulation.scenario.clone(),
            status: simulation.status,
            snapshot: simulation.snapshot.clone(),
            server: server.clone(),
            human_id,
            tool_definitions,
            max_calls: DEFAULT_NATIVE_TOOL_CALL_BUDGET,
            max_cost: DEFAULT_NATIVE_TOOL_COST_BUDGET,
            expires_at_unix_ms: unix_time_ms().saturating_add(lifetime_ms),
            calls: Vec::new(),
        }
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        private_state_platform_guard()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let payload = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let temporary = path.with_extension(format!("tmp-{}", self.generation));
        #[cfg(unix)]
        {
            let mut options = OpenOptions::new();
            options.create_new(true).write(true).mode(0o600);
            let mut file = options
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            file.write_all(&payload)
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            drop(file);
            if let Err(error) = fs::rename(&temporary, path) {
                let _ = fs::remove_file(&temporary);
                return Err(error.to_string());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            let mut file = windows_private_state::create_owner_only(&temporary)?;
            file.write_all(&payload)
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            drop(file);
            if let Err(error) = windows_private_state::replace_atomically(&temporary, path) {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
            windows_private_state::verify_owner_only(path)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (path, payload, temporary);
            unreachable!("unsupported platforms fail in private_state_platform_guard")
        }
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        private_state_platform_guard()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path)
                .map_err(|error| error.to_string())?
                .permissions()
                .mode()
                & 0o777;
            if mode != 0o600 {
                return Err(format!(
                    "Native MCP state permissions must be 0600, found {mode:04o}"
                ));
            }
        }
        #[cfg(windows)]
        windows_private_state::verify_owner_only(path)?;
        let payload = fs::read(path).map_err(|error| error.to_string())?;
        let state: Self = serde_json::from_slice(&payload).map_err(|error| error.to_string())?;
        if state.protocol_version != NATIVE_MCP_PROTOCOL_VERSION {
            return Err(format!(
                "native MCP state protocol {} is incompatible with {}",
                state.protocol_version, NATIVE_MCP_PROTOCOL_VERSION
            ));
        }
        Ok(state)
    }

    pub fn into_calls(self) -> Vec<NativeMcpCall> {
        self.calls
    }
}

pub fn run_stdio(state_path: PathBuf) -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => break,
            Err(error) => return Err(error.to_string()),
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if request.get("id").is_none() {
            continue;
        }
        let response = handle_request(&state_path, &request);
        match writeln!(stdout, "{}", response) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => break,
            Err(error) => return Err(error.to_string()),
        }
        stdout.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn handle_request(state_path: &Path, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => rpc_ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": { "name": "cockpit-world", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "tools/list" => match NativeMcpTurnState::read(state_path) {
            Ok(state) => rpc_ok(id, json!({ "tools": state.tool_definitions })),
            Err(error) => rpc_error(id, -32001, &format!("turn state unavailable: {error}")),
        },
        "tools/call" => handle_tool_call(state_path, id, request),
        method => rpc_error(id, -32601, &format!("unknown method {method}")),
    }
}

fn handle_tool_call(state_path: &Path, id: Value, request: &Value) -> Value {
    let mut state = match NativeMcpTurnState::read(state_path) {
        Ok(state) => state,
        Err(error) => return rpc_error(id, -32001, &format!("turn state unavailable: {error}")),
    };
    if unix_time_ms() > state.expires_at_unix_ms {
        return tool_error(
            id,
            "TURN_BUDGET_EXPIRED",
            "native tool turn deadline expired",
        );
    }
    let params = request.get("params").unwrap_or(&Value::Null);
    let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if state
        .calls
        .iter()
        .any(|call| call.tool == crate::TOOL_SUBMIT_DECISION && call.response.error.is_none())
    {
        return tool_error(
            id,
            "DECISION_ALREADY_SUBMITTED",
            "simulation.submit_decision must be the final native tool call",
        );
    }
    let side_effect = match state
        .tool_definitions
        .iter()
        .find(|definition| definition.name == tool_name)
    {
        Some(definition) => definition.side_effect,
        None => {
            return tool_error(
                id,
                "TOOL_NOT_AUTHORIZED",
                "tool is not available in this human turn",
            );
        }
    };
    if state.calls.len() >= state.max_calls {
        return tool_error(
            id,
            "TOOL_CALL_BUDGET_EXCEEDED",
            "native tool call budget exhausted",
        );
    }
    let spent = state
        .calls
        .iter()
        .map(|call| tool_cost(&call.tool))
        .sum::<u32>();
    if spent.saturating_add(tool_cost(tool_name)) > state.max_cost {
        return tool_error(
            id,
            "TOOL_COST_BUDGET_EXCEEDED",
            "native tool cost budget exhausted",
        );
    }

    let mut simulation = Simulation::from_tool_snapshot(
        state.scenario.clone(),
        state.status,
        state.snapshot.clone(),
    );
    let mut server = state.server.clone();
    for prior in &state.calls {
        let prior_request = scoped_request(
            &state,
            prior.call_id.clone(),
            &prior.tool,
            prior.arguments.clone(),
        );
        let _ = server.call(&mut simulation, prior_request);
    }

    let call_id = format!("{}-native-{}", state.generation, state.calls.len() + 1);
    let tool_request = scoped_request(&state, call_id.clone(), tool_name, arguments.clone());
    let (response, _) = server.call(&mut simulation, tool_request);
    let is_error = response.error.is_some();
    state.calls.push(NativeMcpCall {
        call_id,
        tool: tool_name.to_string(),
        arguments: redact_json(arguments),
        response: response.clone(),
    });
    if let Err(error) = state.write(state_path) {
        return rpc_error(
            id,
            -32002,
            &format!("failed to persist native tool trace: {error}"),
        );
    }
    let structured = serde_json::to_value(&response).unwrap_or(Value::Null);
    rpc_ok(
        id,
        json!({
            "content": [{ "type": "text", "text": structured.to_string() }],
            "structuredContent": structured,
            "isError": is_error,
            "sideEffect": side_effect
        }),
    )
}

fn scoped_request(
    state: &NativeMcpTurnState,
    call_id: String,
    tool_name: &str,
    arguments: Value,
) -> ToolRequest {
    ToolRequest {
        correlation_id: format!("{call_id}-corr"),
        call_id,
        run_id: state.snapshot.run_id.clone(),
        agent_id: state.scenario.agent.agent_id.clone(),
        human_id: Some(state.human_id.clone()),
        tick: state.snapshot.tick,
        tool_name: tool_name.to_string(),
        arguments,
    }
}

fn tool_cost(tool_name: &str) -> u32 {
    match tool_name {
        crate::TOOL_SUBMIT_DECISION => 0,
        crate::TOOL_REQUEST_ACTION => 4,
        crate::TOOL_ADD_GOAL | crate::TOOL_WAIT_UNTIL => 2,
        _ => 1,
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn tool_error(id: Value, code: &str, message: &str) -> Value {
    rpc_ok(
        id,
        json!({
            "content": [{ "type": "text", "text": message }],
            "structuredContent": { "error": { "code": code, "message": message } },
            "isError": true
        }),
    )
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_sddl_accepts_only_one_protected_owner_rights_ace() {
        assert!(is_owner_only_windows_sddl("D:P(A;;FA;;;OW)"));
        assert!(is_owner_only_windows_sddl("D:P(A;;FA;;;S-1-3-4)"));
        assert!(!is_owner_only_windows_sddl("D:(A;;FA;;;OW)"));
        assert!(!is_owner_only_windows_sddl("D:P(A;;FA;;;OW)(A;;FR;;;BU)"));
        assert!(!is_owner_only_windows_sddl("D:P(A;;FA;;;WD)"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_state_file_has_verified_owner_only_dacl() {
        let path = std::env::temp_dir().join(format!(
            "cockpit-native-acl-test-{}-{}.json",
            std::process::id(),
            unix_time_ms()
        ));
        let mut file = windows_private_state::create_owner_only(&path).expect("secure create");
        file.write_all(b"{}").expect("write fixture");
        file.sync_all().expect("sync fixture");
        drop(file);
        windows_private_state::verify_owner_only(&path).expect("owner-only ACL verifies");
        let _ = fs::remove_file(path);
    }

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn native_mcp_private_state_fails_closed_without_owner_only_permissions() {
        let error = private_state_platform_guard().expect_err("platform must fail closed");
        assert!(error.contains("owner-only"));
    }

    #[cfg(unix)]
    #[test]
    fn native_bridge_lists_and_executes_scoped_tools() {
        let scenario_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/smoke-in-cockpit.yaml");
        let scenario = cockpit_scenario::load_scenario(
            scenario_path.to_str().expect("scenario path is UTF-8"),
        )
        .expect("scenario loads");
        let mut simulation = Simulation::new("native-contract-run", scenario);
        simulation.start().expect("simulation starts");
        let state_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../.native-mcp-test-{}.json",
            std::process::id()
        ));
        let state = NativeMcpTurnState::new(
            "generation-1".to_string(),
            &simulation,
            &LocalMcpServer::default(),
            "pilot-1".to_string(),
            LocalMcpServer::tool_definitions(),
            30_000,
        );
        state.write(&state_path).expect("state writes");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&state_path)
                .expect("state metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let listed = handle_request(
            &state_path,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        assert_eq!(listed["result"]["tools"].as_array().map(Vec::len), Some(9));

        let called = handle_request(
            &state_path,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": crate::TOOL_GET_RUN_STATUS, "arguments": {} }
            }),
        );
        assert_eq!(called["result"]["isError"], false);
        let persisted = NativeMcpTurnState::read(&state_path).expect("state reads");
        assert_eq!(persisted.calls.len(), 1);
        assert_eq!(persisted.calls[0].tool, crate::TOOL_GET_RUN_STATUS);

        let submitted = handle_request(
            &state_path,
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": crate::TOOL_SUBMIT_DECISION,
                    "arguments": {
                        "utterance": null,
                        "internalStateDelta": { "stress": null, "attention": 0.1 },
                        "narrative": "I stay alert."
                    }
                }
            }),
        );
        assert_eq!(submitted["result"]["isError"], false);
        let after_submission = handle_request(
            &state_path,
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": crate::TOOL_GET_RUN_STATUS, "arguments": {} }
            }),
        );
        assert_eq!(after_submission["result"]["isError"], true);
        assert_eq!(
            after_submission["result"]["structuredContent"]["error"]["code"],
            "DECISION_ALREADY_SUBMITTED"
        );

        let _ = fs::remove_file(state_path);
    }
}
