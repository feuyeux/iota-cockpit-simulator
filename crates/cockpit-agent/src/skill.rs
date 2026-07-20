pub const COCKPIT_SKILL_NAME: &str = "cockpit-world";
pub const COCKPIT_SKILL_VERSION: &str = "6";

pub fn required_tools() -> [&'static str; 10] {
    [
        "simulation.get_turn_context",
        "simulation.get_observation",
        "simulation.list_visible_entities",
        "simulation.inspect_sensor_quality",
        "simulation.request_action",
        "simulation.get_action_result",
        "simulation.get_run_status",
        "simulation.add_goal",
        "simulation.wait_until",
        "simulation.submit_decision",
    ]
}
