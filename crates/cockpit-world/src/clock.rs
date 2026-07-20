use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClockMode {
    Realtime,
    Accelerated,
    Stepped,
    Replay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockConfig {
    pub mode: ClockMode,
    pub tick_ms: u64,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            mode: ClockMode::Stepped,
            tick_ms: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Created,
    Validating,
    Ready,
    Running,
    Paused,
    Degraded,
    Replaying,
    Completed,
    Stopped,
    Failed,
}
