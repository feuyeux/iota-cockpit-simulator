use std::future::Future;

use cockpit_simulation_core::{
    StateDiff,
    error::SimulationResult,
    simulation::{Simulation, StepRecord},
};

use crate::{
    LocalMcpServer, RuleAgent,
    policy::{AgentRuntimePolicy, TurnDisposition},
};

/// Drives one deterministic tick while recording the disposition of a live
/// agent turn as per-tick evidence.
///
/// The live turn is advisory: it runs through the retry/circuit-breaker policy
/// and never bypasses the Action Gateway. Authorized tool calls remain
/// deterministic and are still executed by the [`RuleAgent`], so replay hashes
/// stay stable regardless of whether the external backend completed, degraded,
/// or fell back.
pub struct LiveAgentDriver {
    policy: AgentRuntimePolicy,
    agent: RuleAgent,
}

impl LiveAgentDriver {
    pub fn new(policy: AgentRuntimePolicy) -> Self {
        Self {
            policy,
            agent: RuleAgent::default(),
        }
    }

    /// Run a live agent turn through the runtime policy, then commit the
    /// deterministic tick. The turn source is invoked per attempt so the retry
    /// and circuit-breaker policy can re-issue it; its disposition is recorded
    /// on the returned [`StepRecord`].
    pub async fn step<F, Fut, E>(
        &mut self,
        simulation: &mut Simulation,
        server: &mut LocalMcpServer,
        plugin_diffs: Vec<StateDiff>,
        run_turn: F,
    ) -> SimulationResult<StepRecord>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, E>>,
        E: ToString,
    {
        let turn = self
            .policy
            .execute_retrying(run_turn, || "rule-agent-fallback".to_string())
            .await;
        let mut step = self
            .agent
            .step_with_state_diffs(simulation, server, plugin_diffs)?;
        step.fallback = Some(disposition_label(&turn.disposition, turn.elapsed_ms));
        Ok(step)
    }

    /// Like [`step`](Self::step), but the live turn can be cancelled mid-flight
    /// via `cancel`. `is_cancelled` classifies an operation error as a
    /// deliberate cancellation. A cancelled turn is recorded as a
    /// `cancelled:...` disposition; the deterministic tick is still committed.
    pub async fn step_cancellable<F, Fut, E>(
        &mut self,
        simulation: &mut Simulation,
        server: &mut LocalMcpServer,
        plugin_diffs: Vec<StateDiff>,
        run_turn: F,
        is_cancelled: impl Fn(&E) -> bool,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> SimulationResult<StepRecord>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, E>>,
        E: ToString,
    {
        let turn = self
            .policy
            .execute_cancellable(run_turn, is_cancelled, cancel, || {
                "rule-agent-fallback".to_string()
            })
            .await;
        let mut step = self
            .agent
            .step_with_state_diffs(simulation, server, plugin_diffs)?;
        step.fallback = Some(disposition_label(&turn.disposition, turn.elapsed_ms));
        Ok(step)
    }
}

/// Stable, redaction-safe label describing how a live agent turn resolved.
///
/// The label is recorded per tick so recordings and runner events carry
/// authoritative completed/fallback/degraded evidence without leaking prompts
/// or hidden reasoning.
pub fn disposition_label(disposition: &TurnDisposition, elapsed_ms: u64) -> String {
    match disposition {
        TurnDisposition::Completed => format!("completed:{elapsed_ms}ms"),
        TurnDisposition::Fallback { policy, reason } => {
            format!("fallback:{policy:?}:{reason}:{elapsed_ms}ms")
        }
        TurnDisposition::Cancelled { reason } => {
            format!("cancelled:{reason}:{elapsed_ms}ms")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use cockpit_scenario::load_scenario;
    use cockpit_simulation_core::Simulation;

    use super::*;
    use crate::FallbackPolicy;

    fn started_simulation() -> Simulation {
        let scenario = load_scenario("../../scenarios/smoke-in-cockpit.yaml")
            .expect("smoke scenario loads for the live driver test");
        let mut simulation = Simulation::new("live-driver-run", scenario);
        simulation.start().expect("simulation starts");
        simulation
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_turn_records_completed_disposition() {
        let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(2, 3);
        let mut driver = LiveAgentDriver::new(policy);
        let mut simulation = started_simulation();
        let mut server = LocalMcpServer::default();

        let step = driver
            .step(&mut simulation, &mut server, Vec::new(), || async {
                Ok::<_, &'static str>("shutdown the engine".to_string())
            })
            .await
            .expect("live driver commits a deterministic tick");

        let fallback = step.fallback.expect("disposition is recorded per tick");
        assert!(fallback.starts_with("completed:"), "unexpected: {fallback}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_turn_retries_then_records_fallback_evidence() {
        let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(2, 3);
        let mut driver = LiveAgentDriver::new(policy);
        let mut simulation = started_simulation();
        let mut server = LocalMcpServer::default();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);

        let step = driver
            .step(&mut simulation, &mut server, Vec::new(), move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Err::<String, _>("backend unavailable") }
            })
            .await
            .expect("live driver still commits the deterministic tick");

        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "the policy retries the failing turn before falling back"
        );
        let fallback = step.fallback.expect("disposition is recorded per tick");
        assert!(fallback.starts_with("fallback:"), "unexpected: {fallback}");
        assert!(fallback.contains("backend unavailable"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_turn_records_cancelled_disposition_and_still_commits() {
        let policy = AgentRuntimePolicy::new(50, 1, FallbackPolicy::RuleAgent).with_retry(3, 3);
        let mut driver = LiveAgentDriver::new(policy);
        let mut simulation = started_simulation();
        let mut server = LocalMcpServer::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();

        let tick_before = simulation.snapshot.tick;
        let step = driver
            .step_cancellable(
                &mut simulation,
                &mut server,
                Vec::new(),
                || async { Ok::<_, &'static str>("ignored".to_string()) },
                |_error| false,
                &cancel,
            )
            .await
            .expect("cancelled turn still commits the deterministic tick");

        let fallback = step.fallback.expect("disposition is recorded per tick");
        assert!(fallback.starts_with("cancelled:"), "unexpected: {fallback}");
        assert_eq!(
            simulation.snapshot.tick,
            tick_before + 1,
            "the deterministic tick is committed even when the live turn is cancelled"
        );
    }
}
