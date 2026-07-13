use cockpit_agent_runtime::{AgentRuntimePolicy, FallbackPolicy, LiveAgentDriver, LocalMcpServer};
use cockpit_recording::Recording;
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::{Simulation, clock::RunStatus};
use serde::Serialize;
use serde_json::Value;

/// Configuration for a live-agent run driven through the runtime policy.
#[derive(Debug, Clone)]
pub struct LiveRunConfig {
    pub scenario_path: String,
    pub ticks: u64,
    pub timeout_ms: u64,
    pub max_attempts: usize,
    pub circuit_failure_threshold: usize,
}

/// Per-tick disposition evidence for a live run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTickEvidence {
    pub tick: u64,
    pub snapshot_hash: String,
    pub disposition: String,
}

/// Result of a live-agent run, including authoritative per-tick disposition
/// evidence and the final deterministic snapshot hash.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRunReport {
    pub run_id: String,
    pub scenario_hash: String,
    pub ticks: usize,
    pub final_snapshot_hash: Option<String>,
    pub completed_turns: usize,
    pub fallback_turns: usize,
    pub tick_evidence: Vec<LiveTickEvidence>,
    pub backend: &'static str,
    pub evaluation: Value,
}

/// Drive a live-agent run for `config.ticks` ticks.
///
/// Every tick runs an advisory live agent turn through the retry/circuit-breaker
/// policy with a RuleAgent fallback; the deterministic tick is always committed
/// and its disposition recorded, so the run stays replayable regardless of
/// backend health.
pub async fn run_live(config: LiveRunConfig) -> anyhow::Result<LiveRunReport> {
    let scenario = load_scenario(&config.scenario_path)?;
    let deadline = scenario.shutdown_deadline_ticks;
    let run_id = format!("live-run-{}", scenario.id);
    let mut simulation = Simulation::new(run_id.clone(), scenario.clone());
    simulation.start()?;
    let mut server = LocalMcpServer::default();
    let mut recording = Recording::new(run_id.clone(), &scenario);

    let policy = AgentRuntimePolicy::new(config.timeout_ms, 1, FallbackPolicy::RuleAgent)
        .with_retry(config.max_attempts, config.circuit_failure_threshold);
    let mut driver = LiveAgentDriver::new(policy);

    let mut backend = backend_session(&scenario, config.timeout_ms)?;
    let mut evidence = Vec::with_capacity(config.ticks as usize);
    let mut completed_turns = 0usize;
    let mut fallback_turns = 0usize;

    for _ in 0..config.ticks {
        if simulation.status != RunStatus::Running {
            break;
        }
        let observation = simulation.observation();
        // Capture observation by value in the closure to avoid borrowing backend
        let obs_clone = observation.clone();
        let mut call_count = 0;
        let backend_ptr = &mut backend as *mut BackendSession;
        let step = driver
            .step(&mut simulation, &mut server, Vec::new(), || {
                call_count += 1;
                let obs = obs_clone.clone();
                async move {
                    // SAFETY: backend_ptr is valid for the duration of this loop iteration
                    // and the closure is only called synchronously within step()
                    let backend_ref = unsafe { &mut *backend_ptr };
                    backend_ref.run_turn(&obs).await
                }
            })
            .await?;
        let disposition = step.fallback.clone().unwrap_or_default();
        if disposition.starts_with("completed:") {
            completed_turns += 1;
        } else {
            fallback_turns += 1;
        }
        evidence.push(LiveTickEvidence {
            tick: step.tick,
            snapshot_hash: step.snapshot_hash.clone(),
            disposition,
        });
        recording.push(step);
    }

    let evaluation = serde_json::to_value(cockpit_evaluation::evaluate_smoke_shutdown(
        &recording, deadline,
    ))?;

    Ok(LiveRunReport {
        run_id,
        scenario_hash: scenario.scenario_hash,
        ticks: recording.ticks.len(),
        final_snapshot_hash: recording.final_snapshot_hash().map(str::to_string),
        completed_turns,
        fallback_turns,
        tick_evidence: evidence,
        backend: backend.label(),
        evaluation,
    })
}

// The backend session abstraction lets the deterministic default build exercise
// the full live driver/policy/recording path without the external iota-core
// process, while the `live-acp` feature swaps in the real ACP backend.

#[cfg(not(feature = "live-acp"))]
mod backend_impl {
    use cockpit_simulation_core::{SimulationScenario, sensor::Observation};

    /// Synthetic backend session used when the real ACP backend is not compiled
    /// in. It returns a fixed advisory decision so the retry/fallback/recording
    /// path runs deterministically and offline.
    pub struct BackendSession;

    impl BackendSession {
        pub async fn run_turn(
            &mut self,
            _observation: &Observation,
        ) -> Result<String, std::convert::Infallible> {
            Ok("advisory: monitor smoke alerts and shut the engine down if detected".to_string())
        }

        pub fn label(&self) -> &'static str {
            "synthetic"
        }
    }

    pub fn backend_session(
        _scenario: &SimulationScenario,
        _timeout_ms: u64,
    ) -> anyhow::Result<BackendSession> {
        Ok(BackendSession)
    }
}

#[cfg(feature = "live-acp")]
mod backend_impl {
    use cockpit_agent_runtime::{
        acp_adapter::{AcpAdapterConfig, IotaCoreAcpAdapter},
        iota_core_adapter::{CockpitSkill, IotaCoreAdapter},
    };
    use cockpit_simulation_core::{SimulationScenario, sensor::Observation};
    use iota_core::config::NimiaConfig;

    /// Live backend session backed by the real iota-core ACP adapter.
    pub struct BackendSession {
        adapter: IotaCoreAcpAdapter,
        skill: CockpitSkill,
    }

    impl BackendSession {
        pub async fn run_turn(&mut self, observation: &Observation) -> Result<String, String> {
            let turn = self
                .adapter
                .execute(observation, &self.skill, || {
                    "rule-agent-fallback".to_string()
                })
                .await
                .map_err(|error| error.to_string())?;
            Ok(turn.text)
        }

        pub fn label(&self) -> &'static str {
            "iota-core-acp"
        }
    }

    pub fn backend_session(
        _scenario: &SimulationScenario,
        timeout_ms: u64,
    ) -> anyhow::Result<BackendSession> {
        let skill = IotaCoreAdapter::new(env!("CARGO_MANIFEST_DIR"))
            .load_cockpit_skill()
            .map_err(anyhow::Error::msg)?;
        let adapter_config = AcpAdapterConfig {
            timeout_ms,
            ..AcpAdapterConfig::default()
        };
        let adapter = IotaCoreAcpAdapter::new(NimiaConfig::default(), adapter_config);
        Ok(BackendSession { adapter, skill })
    }
}

use backend_impl::{BackendSession, backend_session};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn live_run_records_disposition_evidence_per_tick() {
        let report = run_live(LiveRunConfig {
            scenario_path: "../../scenarios/smoke-in-cockpit.yaml".to_string(),
            ticks: 5,
            timeout_ms: 50,
            max_attempts: 2,
            circuit_failure_threshold: 3,
        })
        .await
        .expect("live run completes");

        assert!(report.ticks > 0, "at least one tick is committed");
        assert_eq!(
            report.tick_evidence.len(),
            report.ticks,
            "every committed tick carries disposition evidence"
        );
        assert_eq!(
            report.completed_turns + report.fallback_turns,
            report.ticks,
            "every tick is classified as completed or fallback"
        );
        assert!(report.final_snapshot_hash.is_some());
    }
}
