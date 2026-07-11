use std::{
    future::Future,
    sync::{Arc, Mutex},
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
    pub max_attempts: usize,
    pub circuit_failure_threshold: usize,
    permits: Arc<Semaphore>,
    circuit: Arc<Mutex<CircuitState>>,
}

#[derive(Debug, Default)]
struct CircuitState {
    consecutive_failures: usize,
    open_until: Option<Instant>,
}

impl AgentRuntimePolicy {
    pub fn new(timeout_ms: u64, max_concurrent_sessions: usize, fallback: FallbackPolicy) -> Self {
        Self {
            timeout_ms: timeout_ms.max(1),
            max_concurrent_sessions: max_concurrent_sessions.max(1),
            fallback,
            max_attempts: 1,
            circuit_failure_threshold: 3,
            permits: Arc::new(Semaphore::new(max_concurrent_sessions.max(1))),
            circuit: Arc::new(Mutex::new(CircuitState::default())),
        }
    }

    pub fn with_retry(mut self, max_attempts: usize, circuit_failure_threshold: usize) -> Self {
        self.max_attempts = max_attempts.max(1);
        self.circuit_failure_threshold = circuit_failure_threshold.max(1);
        self
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

    pub async fn execute_retrying<F, Fut, T, E>(
        &self,
        mut operation: F,
        fallback: impl FnOnce() -> T,
    ) -> AgentTurn<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: ToString,
    {
        let started = Instant::now();
        let fallback = &mut Some(fallback);
        if self.circuit_is_open() {
            return AgentTurn::fallback(
                fallback.take().expect("fallback is available")(),
                TurnDisposition::Fallback {
                    policy: self.fallback.clone(),
                    reason: "agent circuit breaker is open".to_string(),
                },
                started.elapsed(),
            );
        }

        let mut last_error = "agent turn failed".to_string();
        for attempt in 1..=self.max_attempts {
            let permit = match timeout(
                Duration::from_millis(self.timeout_ms),
                self.permits.clone().acquire_owned(),
            )
            .await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(error)) => {
                    last_error = format!("runtime permit unavailable: {error}");
                    break;
                }
                Err(_) => {
                    last_error = "agent concurrency permit timed out".to_string();
                    break;
                }
            };
            let result = timeout(Duration::from_millis(self.timeout_ms), operation()).await;
            drop(permit);
            match result {
                Ok(Ok(value)) => {
                    self.record_success();
                    return AgentTurn {
                        value,
                        disposition: TurnDisposition::Completed,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    };
                }
                Ok(Err(error)) => last_error = error.to_string(),
                Err(_) => last_error = format!("agent turn exceeded {}ms", self.timeout_ms),
            }
            if attempt < self.max_attempts {
                continue;
            }
        }
        self.record_failure();
        AgentTurn::fallback(
            fallback.take().expect("fallback is available")(),
            TurnDisposition::Fallback {
                policy: self.fallback.clone(),
                reason: format!("{last_error} after {} attempt(s)", self.max_attempts),
            },
            started.elapsed(),
        )
    }

    fn circuit_is_open(&self) -> bool {
        let mut circuit = self
            .circuit
            .lock()
            .expect("agent circuit lock is available");
        match circuit.open_until {
            Some(deadline) if deadline > Instant::now() => true,
            Some(_) => {
                circuit.open_until = None;
                circuit.consecutive_failures = 0;
                false
            }
            None => false,
        }
    }

    fn record_success(&self) {
        let mut circuit = self
            .circuit
            .lock()
            .expect("agent circuit lock is available");
        circuit.consecutive_failures = 0;
        circuit.open_until = None;
    }

    fn record_failure(&self) {
        let mut circuit = self
            .circuit
            .lock()
            .expect("agent circuit lock is available");
        circuit.consecutive_failures += 1;
        if circuit.consecutive_failures >= self.circuit_failure_threshold {
            circuit.open_until = Some(Instant::now() + Duration::from_millis(self.timeout_ms));
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
