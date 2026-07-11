use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::{sync::Semaphore, time::timeout};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FallbackPolicy {
    RuleAgent,
    PauseRun,
    FailRun,
}

#[derive(Debug, Clone)]
pub struct AgentRuntimePolicy {
    pub timeout_ms: u64,
    pub max_concurrent_sessions: usize,
    pub fallback: FallbackPolicy,
    permits: Arc<Semaphore>,
}

impl AgentRuntimePolicy {
    pub fn new(timeout_ms: u64, max_concurrent_sessions: usize, fallback: FallbackPolicy) -> Self {
        Self {
            timeout_ms: timeout_ms.max(1),
            max_concurrent_sessions: max_concurrent_sessions.max(1),
            fallback,
            permits: Arc::new(Semaphore::new(max_concurrent_sessions.max(1))),
        }
    }

    pub async fn execute<F, T, E>(&self, operation: F, fallback: impl FnOnce() -> T) -> AgentTurn<T>
    where
        F: Future<Output = Result<T, E>>,
        E: ToString,
    {
        let started = Instant::now();
        let fallback = &mut Some(fallback);
        let permit = match timeout(
            Duration::from_millis(self.timeout_ms),
            self.permits.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(error)) => {
                return AgentTurn::fallback(
                    fallback.take().expect("fallback is available")(),
                    TurnDisposition::Fallback {
                        policy: self.fallback.clone(),
                        reason: format!("runtime permit unavailable: {error}"),
                    },
                    started.elapsed(),
                );
            }
            Err(_) => {
                return AgentTurn::fallback(
                    fallback.take().expect("fallback is available")(),
                    TurnDisposition::Fallback {
                        policy: self.fallback.clone(),
                        reason: "agent concurrency permit timed out".to_string(),
                    },
                    started.elapsed(),
                );
            }
        };

        let result = timeout(Duration::from_millis(self.timeout_ms), operation).await;
        drop(permit);
        match result {
            Ok(Ok(value)) => AgentTurn {
                value,
                disposition: TurnDisposition::Completed,
                elapsed_ms: started.elapsed().as_millis() as u64,
            },
            Ok(Err(error)) => AgentTurn::fallback(
                fallback.take().expect("fallback is available")(),
                TurnDisposition::Fallback {
                    policy: self.fallback.clone(),
                    reason: error.to_string(),
                },
                started.elapsed(),
            ),
            Err(_) => AgentTurn::fallback(
                fallback.take().expect("fallback is available")(),
                TurnDisposition::Fallback {
                    policy: self.fallback.clone(),
                    reason: format!("agent turn exceeded {}ms", self.timeout_ms),
                },
                started.elapsed(),
            ),
        }
    }
}

impl Default for AgentRuntimePolicy {
    fn default() -> Self {
        Self::new(2_000, 1, FallbackPolicy::RuleAgent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnDisposition {
    Completed,
    Fallback {
        policy: FallbackPolicy,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTurn<T> {
    pub value: T,
    pub disposition: TurnDisposition,
    pub elapsed_ms: u64,
}

impl<T> AgentTurn<T> {
    fn fallback(value: T, disposition: TurnDisposition, elapsed: Duration) -> Self {
        Self {
            value,
            disposition,
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }
}
