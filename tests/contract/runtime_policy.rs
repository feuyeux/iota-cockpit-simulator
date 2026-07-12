use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use cockpit_agent_runtime::{AgentRuntimePolicy, FallbackPolicy, TurnDisposition};
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "current_thread")]
async fn timed_out_agent_turn_returns_explicit_rule_fallback() {
    let policy = AgentRuntimePolicy::new(5, 1, FallbackPolicy::RuleAgent);
    let turn = policy
        .execute(
            async {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok::<_, &'static str>("external-agent")
            },
            || "rule-agent",
        )
        .await;

    assert_eq!(turn.value, "rule-agent");
    assert!(matches!(turn.disposition, TurnDisposition::Fallback { .. }));
    assert!(turn.elapsed_ms < 1000);
}

#[tokio::test(flavor = "current_thread")]
async fn failed_agent_turn_retries_before_using_fallback() {
    let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(2, 3);
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let turn = policy
        .execute_retrying(
            move || {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        Err::<&'static str, _>("transient failure")
                    } else {
                        Ok("external-agent")
                    }
                }
            },
            || "rule-agent",
        )
        .await;
    assert_eq!(turn.value, "external-agent");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert!(matches!(turn.disposition, TurnDisposition::Completed));
}

#[tokio::test(flavor = "current_thread")]
async fn repeated_failures_open_the_agent_circuit() {
    let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(1, 2);
    for _ in 0..2 {
        let turn = policy
            .execute_retrying(|| async { Err::<(), _>("backend unavailable") }, || ())
            .await;
        assert!(matches!(turn.disposition, TurnDisposition::Fallback { .. }));
    }
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let turn = policy
        .execute_retrying(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok::<_, &'static str>(()) }
            },
            || (),
        )
        .await;
    assert!(matches!(turn.disposition, TurnDisposition::Fallback { .. }));
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn pre_cancelled_token_skips_the_operation() {
    let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(3, 3);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let turn = policy
        .execute_cancellable(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok::<_, &'static str>("external-agent") }
            },
            |_error| false,
            &cancel,
            || "rule-agent",
        )
        .await;
    assert_eq!(turn.value, "rule-agent");
    assert!(matches!(turn.disposition, TurnDisposition::Cancelled { .. }));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        0,
        "a pre-cancelled turn never invokes the backend"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn backend_reported_cancellation_is_terminal_and_not_retried() {
    let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(3, 3);
    let cancel = CancellationToken::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let turn = policy
        .execute_cancellable(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Err::<&'static str, _>("turn cancelled") }
            },
            |error| error.contains("cancelled"),
            &cancel,
            || "rule-agent",
        )
        .await;
    assert!(matches!(turn.disposition, TurnDisposition::Cancelled { .. }));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "a cancelled turn is terminal: it is not retried"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn token_fired_mid_flight_cancels_the_turn() {
    let policy = AgentRuntimePolicy::new(1_000, 1, FallbackPolicy::RuleAgent);
    let cancel = CancellationToken::new();
    let child = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        child.cancel();
    });
    let turn = policy
        .execute_cancellable(
            || async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok::<_, &'static str>("external-agent")
            },
            |_error| false,
            &cancel,
            || "rule-agent",
        )
        .await;
    assert_eq!(turn.value, "rule-agent");
    assert!(matches!(turn.disposition, TurnDisposition::Cancelled { .. }));
    assert!(turn.elapsed_ms < 400, "cancellation returns promptly");
}
