mod runner_commands;

use runner_commands::RunnerState;
use std::path::PathBuf;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let token = format!(
        "cockpit-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    
    // Capture workspace root at startup
    // In dev mode, CARGO_MANIFEST_DIR points to apps/cockpit-desktop/src-tauri
    // We need to go up to the repository root (../../..)
    let workspace_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .and_then(|manifest_dir| {
            // Go up from apps/cockpit-desktop/src-tauri to project root
            manifest_dir.parent()?.parent()?.parent().map(|p| p.to_path_buf())
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    
    eprintln!("Cockpit Desktop: workspace_root = {}", workspace_root.display());
    
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(RunnerState::new(token, workspace_root))
        .invoke_handler(tauri::generate_handler![
            runner_commands::connect_runner,
            runner_commands::validate_scenario,
            runner_commands::create_simulation_run,
            runner_commands::start_simulation,
            runner_commands::pause_simulation,
            runner_commands::step_simulation,
            runner_commands::stop_simulation,
            runner_commands::resume_simulation,
            runner_commands::approve_action,
            runner_commands::reject_action,
            runner_commands::cancel_agent_turn,
            runner_commands::set_approval_required,
            runner_commands::start_replay,
            runner_commands::diff_recordings,
            runner_commands::get_simulation_events,
            runner_commands::get_simulation_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running cockpit desktop");
}
