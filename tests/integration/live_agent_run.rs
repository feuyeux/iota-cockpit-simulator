//! End-to-end coverage for the live-agent simulator path.
//!
//! The default build exercises the full per-human driver -> synthetic backend
//! -> recording pipeline, so it stays deterministic and offline. There is no
//! fallback: a backend failure must abort the run rather than substitute a
//! value. The `live-acp` feature adds an opt-in test that starts the real
//! iota-core ACP backend against a deliberately short timeout and asserts the
//! run fails fast rather than silently degrading.

use cockpit_recording::{CURRENT_RUNTIME_CONTRACT_VERSION, Recording};
use cockpit_scenario::load_scenario;
use cockpit_simulator::{LiveRunConfig, replay_live, run_live};

fn base_config() -> LiveRunConfig {
    LiveRunConfig {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        ticks: 20,
        timeout_ms: 100,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn live_run_with_the_synthetic_backend_stays_deterministic() {
    let first = run_live(base_config()).await.expect("first live run");

    if first.backend == "iota-core-acp" {
        assert!(
            first.error.is_some(),
            "feature-unified tests must fail closed when no ACP backend is available"
        );
        assert_eq!(
            first.ticks, 0,
            "a failed model turn must not commit its tick"
        );
        return;
    }

    assert_eq!(first.backend, "synthetic");
    let second = run_live(base_config()).await.expect("second live run");
    assert!(first.ticks > 0, "the run commits ticks");
    assert!(first.error.is_none(), "the synthetic backend never fails");
    assert_eq!(
        first.tick_evidence.len(),
        first.ticks,
        "every committed tick carries its event-driven decision evidence"
    );
    assert!(
        first
            .tick_evidence
            .iter()
            .any(|tick| tick.humans.is_empty()),
        "idle ticks must not spend a synthetic backend turn"
    );
    assert!(
        first
            .tick_evidence
            .iter()
            .any(|tick| !tick.humans.is_empty()),
        "initial, cadence, or event ticks must retain decision evidence"
    );
    let scheduled_turns = first
        .tick_evidence
        .iter()
        .map(|tick| tick.humans.len())
        .sum::<usize>();
    assert!(
        scheduled_turns < first.ticks as usize * 2,
        "event-driven scheduling must use fewer turns than every-tick, every-human execution"
    );
    for human in first
        .tick_evidence
        .iter()
        .flat_map(|tick| tick.humans.iter())
    {
        assert!(
            !human.tool_calls.is_empty(),
            "the synthetic backend must exercise on-demand simulation tools"
        );
    }
    assert_eq!(
        first.final_snapshot_hash, second.final_snapshot_hash,
        "two runs against the same scenario/backend produce the same final hash"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn live_replay_rejects_the_previous_runtime_contract() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario");
    let mut recording = Recording::new("old-live-run", &scenario);
    recording.runtime_contract_version = CURRENT_RUNTIME_CONTRACT_VERSION - 1;

    let error = replay_live(scenario, &recording)
        .await
        .expect_err("old live recording must be rejected");
    assert!(error.to_string().contains("runtime contract version"));
}

/// Startup + failure handling against the real ACP backend. Opt-in because it
/// starts an external process; deterministic CI runs without the feature.
///
/// A very short timeout with no backend process present must make the run
/// fail outright: there is no fallback/circuit-breaker path that would let the
/// run keep committing ticks without a real backend decision.
#[cfg(feature = "live-acp")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_acp_backend_unavailable_aborts_the_run_without_a_fallback() {
    let report = run_live(LiveRunConfig {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        ticks: 5,
        timeout_ms: 50,
    })
    .await
    .expect("run_live resolves (with a reported error) rather than panicking");

    assert_eq!(report.backend, "iota-core-acp");
    assert!(
        report.error.is_some(),
        "an unavailable backend must be reported as a run failure, not silently degraded"
    );
    assert!(
        report.ticks < 5,
        "the run must not have committed every requested tick without a real backend"
    );
}

#[cfg(not(feature = "live-acp"))]
#[tokio::test(flavor = "current_thread")]
async fn bundled_live_scenarios_are_either_synthetic_successes_or_fail_closed_acp_runs() {
    const SCENARIOS: &[&str] = &[
        "scenarios/smoke-in-cockpit.yaml",
        "scenarios/heatwave-thermal-comfort.yaml",
        "scenarios/winter-defog-visibility.yaml",
        "scenarios/driver-fatigue-guardian.yaml",
        "scenarios/child-left-behind.yaml",
        "scenarios/medical-emergency.yaml",
        "scenarios/voice-privacy-conflict.yaml",
        "scenarios/ev-range-anxiety.yaml",
        "scenarios/adas-takeover-construction.yaml",
        "scenarios/cybersecurity-anomalous-control.yaml",
    ];

    for path in SCENARIOS {
        let scenario = load_scenario(path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let report = run_live(LiveRunConfig {
            scenario_path: path.to_string(),
            ticks: scenario.max_ticks + 6,
            timeout_ms: 100,
        })
        .await
        .unwrap_or_else(|error| panic!("{path}: {error}"));
        let evaluation: cockpit_evaluation::EvaluationResult =
            serde_json::from_value(report.evaluation).expect("evaluation serializes");

        // Cargo workspace feature unification can enable the simulator's
        // `live-acp` dependency through the desktop crate even when this root
        // package does not have its own `live-acp` feature. In that case an
        // unavailable external backend must fail closed rather than be judged
        // as a synthetic success.
        if report.backend == "iota-core-acp" {
            assert!(report.error.is_some(), "{path}: ACP must fail closed");
            continue;
        }
        assert_eq!(report.backend, "synthetic", "{path}");
        assert!(report.error.is_none(), "{path}: {:?}", report.error);
        assert!(evaluation.passed, "{path}: {}", evaluation.explanation);
    }
}
