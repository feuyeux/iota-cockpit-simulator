mod runner_commands;

use runner_commands::RunnerState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RunnerState::new("cockpit-desktop-session"))
        .invoke_handler(tauri::generate_handler![
            runner_commands::connect_runner,
            runner_commands::validate_scenario,
            runner_commands::create_simulation_run,
            runner_commands::start_simulation,
            runner_commands::pause_simulation,
            runner_commands::step_simulation,
            runner_commands::stop_simulation,
            runner_commands::get_simulation_events,
            runner_commands::get_simulation_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running cockpit desktop");
}
