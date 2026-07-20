use serde::{Deserialize, Serialize};

pub type RunId = String;
pub type EntityId = String;
pub type AgentId = String;
pub type EventId = String;
pub type CorrelationId = String;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StateVersion(pub u64);
