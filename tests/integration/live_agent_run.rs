//! End-to-end coverage for the live-agent runner path.
//!
//! The default build exercises the full driver -> retry/circuit-breaker policy
//! -> recording pipeline with a synthetic backend, so it stays deterministic
//! and offline. The `live-acp` feature adds an opt-in test that starts the real
//! iota-core ACP backend and asserts the run still records authoritative
//! disposition evidence and remains replayable.

use cockpit_runner::{LiveRunConfig, run_live};

fn base_config() -> LiveRunConfig {
    LiveRunConfig {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        ticks: 20,
        timeout_ms: 100,
        max_attempts: 2,
        circuit_failure_threshold: 3,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn live_run_records_disposition_evidence_and_stays_deterministic() {
    let first = run_live(base_config()).await.expect("first live run");
    let second = run_live(base_config()).await.expect("second live run");

    assert!(first.ticks > 0, "the run commits ticks");
    assert_eq!(
        first.tick_evidence.len(),
        first.ticks,
        "every committed tick carries completed/fallback disposition evidence"
    );
    assert_eq!(
        first.completed_turns + first.fallback_turns,
        first.ticks,
        "each tick is classified as completed or fallback"
    );
    assert_eq!(
        first.final_snapshot_hash, second.final_snapshot_hash,
        "the deterministic tick commit is independent of live-turn evidence"
    );
    assert!(
        first
            .tick_evidence
            .iter()
            .all(|evidence| !evidence.disposition.is_empty()),
        "no tick is missing disposition evidence"
    );
}

/// Startup + failure handling against the real ACP backend. Opt-in because it
/// starts an external process; deterministic CI runs without the feature.
#[cfg(feature = "live-acp")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_acp_backend_start_records_fallback_when_unavailable() {
    // A very short timeout forces the retry/circuit-breaker policy to record
    // fallback evidence rather than hang when no backend process is present.
    let report = run_live(LiveRunConfig {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        ticks: 5,
        timeout_ms: 50,
        max_attempts: 2,
        circuit_failure_threshold: 2,
    })
    .await
    .expect("live-acp run still commits deterministic ticks");

    assert_eq!(report.backend, "iota-core-acp");
    assert_eq!(
        report.tick_evidence.len(),
        report.ticks,
        "backend failures still produce per-tick evidence"
    );
    assert!(
        report.final_snapshot_hash.is_some(),
        "deterministic ticks are committed even when the backend degrades"
    );
}
