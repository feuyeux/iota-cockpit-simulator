//! Mandatory backend execution: no fallback, no retry, no circuit breaker.
//!
//! Every human's decision in a live run must come from a real backend
//! (hermes, etc.) turn. There is no `RuleAgent` or synthetic value to fall
//! back to when the backend fails: a timeout, backend error, or malformed
//! output is a fatal error for the run. This intentionally removes the
//! previous advisory/fallback/circuit-breaker design; backend calls are now a
//! required dependency rather than an optional enhancement.
//!
//! Cancellation is kept as a distinct, non-error outcome: a deliberately
//! cancelled turn (e.g. the user stops the run mid-flight) is not a backend
//! failure and must not be treated as one.

use std::{future::Future, time::Duration};

use thiserror::Error;
use tokio::time::timeout;

/// Why a mandatory backend turn did not produce a value.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentTurnError {
    /// The backend call itself returned an error.
    #[error("backend turn failed: {0}")]
    BackendFailed(String),
    /// The backend call did not complete within the configured timeout.
    #[error("backend turn exceeded {timeout_ms}ms")]
    TimedOut { timeout_ms: u64 },
    /// The turn was deliberately cancelled mid-flight. Not a backend failure.
    #[error("backend turn cancelled: {0}")]
    Cancelled(String),
}

impl AgentTurnError {
    /// Whether this outcome represents a deliberate cancellation rather than a
    /// backend failure. Callers use this to distinguish "the run must fail"
    /// from "the run was intentionally stopped".
    pub fn is_cancelled(&self) -> bool {
        matches!(self, AgentTurnError::Cancelled(_))
    }
}

/// Minimal policy: only a timeout and a concurrency bound. No fallback value,
/// no retry count, no circuit breaker; any failure is returned to the caller.
#[derive(Debug, Clone)]
pub struct AgentRuntimePolicy {
    pub timeout_ms: u64,
}

impl AgentRuntimePolicy {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            timeout_ms: timeout_ms.max(1),
        }
    }

    /// Run `operation` once with the configured timeout. Returns `Ok(value)`
    /// on success or `Err(AgentTurnError)` on any failure; the caller is
    /// expected to treat `Err` as fatal for the run (except `Cancelled`,
    /// which the caller may treat as a clean stop).
    pub async fn run<F, T, E>(&self, operation: F) -> Result<T, AgentTurnError>
    where
        F: Future<Output = Result<T, E>>,
        E: ToString,
    {
        match timeout(Duration::from_millis(self.timeout_ms), operation).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(AgentTurnError::BackendFailed(error.to_string())),
            Err(_) => Err(AgentTurnError::TimedOut {
                timeout_ms: self.timeout_ms,
            }),
        }
    }

    /// Run `operation` once, racing it against `cancel`. A firing token before
    /// or during the call yields [`AgentTurnError::Cancelled`] rather than a
    /// backend failure.
    pub async fn run_cancellable<F, T, E>(
        &self,
        operation: F,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<T, AgentTurnError>
    where
        F: Future<Output = Result<T, E>>,
        E: ToString,
    {
        if cancel.is_cancelled() {
            return Err(AgentTurnError::Cancelled(
                "backend turn cancelled before start".to_string(),
            ));
        }
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return Err(AgentTurnError::Cancelled(
                    "backend turn cancelled mid-flight".to_string(),
                ));
            }
            result = timeout(Duration::from_millis(self.timeout_ms), operation) => result,
        };
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                let message = error.to_string();
                if message.starts_with("__CANCELLED__:") {
                    Err(AgentTurnError::Cancelled(
                        message.trim_start_matches("__CANCELLED__:").to_string(),
                    ))
                } else {
                    Err(AgentTurnError::BackendFailed(message))
                }
            }
            Err(_) => Err(AgentTurnError::TimedOut {
                timeout_ms: self.timeout_ms,
            }),
        }
    }
}

impl Default for AgentRuntimePolicy {
    fn default() -> Self {
        Self::new(2_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test(flavor = "current_thread")]
    async fn timed_out_backend_turn_is_a_fatal_error() {
        let policy = AgentRuntimePolicy::new(5);
        let error = policy
            .run(async {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok::<_, &'static str>("external-agent")
            })
            .await
            .expect_err("timeout is fatal, not a fallback");
        assert!(matches!(error, AgentTurnError::TimedOut { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_backend_turn_is_a_fatal_error_without_retry() {
        let policy = AgentRuntimePolicy::new(50);
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let error = policy
            .run(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<&'static str, _>("backend unavailable")
            })
            .await
            .expect_err("failure is fatal");
        assert!(matches!(error, AgentTurnError::BackendFailed(_)));
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "no retry is attempted");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn successful_backend_turn_returns_the_value() {
        let policy = AgentRuntimePolicy::new(50);
        let value = policy
            .run(async { Ok::<_, &'static str>("external-agent") })
            .await
            .expect("succeeds");
        assert_eq!(value, "external-agent");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pre_cancelled_token_skips_the_operation() {
        let policy = AgentRuntimePolicy::new(50);
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let error = policy
            .run_cancellable(
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, &'static str>("external-agent")
                },
                &cancel,
            )
            .await
            .expect_err("cancelled before start");
        assert!(error.is_cancelled());
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn token_fired_mid_flight_cancels_the_turn() {
        let policy = AgentRuntimePolicy::new(1_000);
        let cancel = tokio_util::sync::CancellationToken::new();
        let child = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            child.cancel();
        });
        let error = policy
            .run_cancellable(
                async {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok::<_, &'static str>("external-agent")
                },
                &cancel,
            )
            .await
            .expect_err("cancelled mid-flight");
        assert!(error.is_cancelled());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backend_reported_cancellation_is_classified_as_cancelled() {
        let policy = AgentRuntimePolicy::new(50);
        let cancel = tokio_util::sync::CancellationToken::new();
        let error = policy
            .run_cancellable(
                async { Err::<&'static str, _>("__CANCELLED__:turn cancelled") },
                &cancel,
            )
            .await
            .expect_err("cancellation is not a value");
        assert!(error.is_cancelled());
    }
}
