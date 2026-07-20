use crate::action::ErrorCode;
use thiserror::Error;

pub type SimulationResult<T> = Result<T, SimulationError>;

#[derive(Debug, Error)]
pub enum SimulationError {
    #[error("invalid scenario: {0}")]
    InvalidScenario(String),
    #[error("invalid command for run state")]
    InvalidRunState,
    #[error("action rejected: {0:?}")]
    ActionRejected(ErrorCode),
    #[error("serialization failed: {0}")]
    Serialization(String),
}
