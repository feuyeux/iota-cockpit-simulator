pub const COCKPIT_SKILL_NAME: &str = "cockpit-simulation";
pub const COCKPIT_SKILL_VERSION: &str = "1";

pub fn required_tools() -> [&'static str; 6] {
    [
        "simulation.get_observation",
        "simulation.list_visible_entities",
        "simulation.inspect_sensor_quality",
        "simulation.request_action",
        "simulation.get_action_result",
        "simulation.get_run_status",
    ]
}
