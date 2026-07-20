use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use cockpit_agent::{AgentRuntimePolicy, AgentTurnError};
use tokio_util::sync::CancellationToken;

/// A backend turn that exceeds the timeout is a fatal error, not a fallback
/// value: the mandatory-backend contract has no substitute output.
#[tokio::test(flavor = "current_thread")]
async fn timed_out_backend_turn_is_fatal_not_a_fallback() {
    let policy = AgentRuntimePolicy::new(5);
    let error = policy
        .run(async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok::<_, &'static str>("external-agent")
        })
        .await
        .expect_err("timeout is fatal");
    assert!(matches!(error, AgentTurnError::TimedOut { .. }));
}

/// A failing backend turn is fatal on the first attempt: there is no retry.
#[tokio::test(flavor = "current_thread")]
async fn failed_backend_turn_is_fatal_without_retry() {
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
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "the operation is invoked exactly once; there is no retry"
    );
}

/// A successful backend turn returns the value with no wrapping disposition.
#[tokio::test(flavor = "current_thread")]
async fn successful_backend_turn_returns_the_value_directly() {
    let policy = AgentRuntimePolicy::new(50);
    let value = policy
        .run(async { Ok::<_, &'static str>("external-agent") })
        .await
        .expect("succeeds");
    assert_eq!(value, "external-agent");
}

/// Repeated failures do not open a circuit breaker (there is none): each call
/// is independent and always attempts the backend.
#[tokio::test(flavor = "current_thread")]
async fn repeated_failures_do_not_trip_any_circuit_breaker() {
    let policy = AgentRuntimePolicy::new(50);
    for _ in 0..5 {
        let error = policy
            .run(async { Err::<(), _>("backend unavailable") })
            .await
            .expect_err("every call fails independently");
        assert!(matches!(error, AgentTurnError::BackendFailed(_)));
    }
    // A subsequent successful call still succeeds; no breaker state persists.
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    policy
        .run(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, &'static str>(())
        })
        .await
        .expect("succeeds even after prior failures");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

/// A pre-cancelled token skips invoking the operation entirely and reports a
/// distinct `Cancelled` outcome, not a backend failure.
#[tokio::test(flavor = "current_thread")]
async fn pre_cancelled_token_skips_the_operation() {
    let policy = AgentRuntimePolicy::new(50);
    let cancel = CancellationToken::new();
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
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        0,
        "a pre-cancelled turn never invokes the backend"
    );
}

/// A token firing mid-flight cancels the turn promptly, distinct from a
/// backend failure.
#[tokio::test(flavor = "current_thread")]
async fn token_fired_mid_flight_cancels_the_turn() {
    let policy = AgentRuntimePolicy::new(1_000);
    let cancel = CancellationToken::new();
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

/// A backend-reported cancellation (tagged `__CANCELLED__:`) is classified as
/// `Cancelled`, distinct from an ordinary backend failure, and is not retried
/// (there is no retry mechanism at all under the mandatory-backend contract).
#[tokio::test(flavor = "current_thread")]
async fn backend_reported_cancellation_is_terminal_and_distinct_from_failure() {
    let policy = AgentRuntimePolicy::new(50);
    let cancel = CancellationToken::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let error = policy
        .run_cancellable(
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<&'static str, _>("__CANCELLED__:turn cancelled")
            },
            &cancel,
        )
        .await
        .expect_err("cancellation is not a value");
    assert!(error.is_cancelled());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
