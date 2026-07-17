use cockpit_agent_runtime::{HumanAgentDriver, HumanTurnEvidence};
use cockpit_recording::Recording;
use cockpit_scenario::load_scenario;
use cockpit_simulation_core::{Simulation, clock::RunStatus};
use serde::Serialize;
use serde_json::Value;

/// Configuration for a live-agent run. Every human's decision each tick must
/// come from a real backend turn; there is no fallback, retry, or circuit
/// breaker. A backend failure aborts the run immediately.
#[derive(Debug, Clone)]
pub struct LiveRunConfig {
    pub scenario_path: String,
    pub ticks: u64,
    pub timeout_ms: u64,
}

/// Per-tick, per-human disposition evidence for a live run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTickEvidence {
    pub tick: u64,
    pub snapshot_hash: String,
    pub humans: Vec<HumanTurnEvidence>,
}

/// Result of a live-agent run. `ticks` is the number of ticks committed
/// before either completing the requested tick count or the run being aborted
/// by a fatal backend error (in which case `error` is set and `ticks` is the
/// count of ticks successfully committed beforehand).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRunReport {
    pub run_id: String,
    pub scenario_hash: String,
    pub ticks: usize,
    pub final_snapshot_hash: Option<String>,
    pub tick_evidence: Vec<LiveTickEvidence>,
    pub backend: &'static str,
    pub evaluation: Value,
    /// Set when the run was aborted by a mandatory backend failure. `None`
    /// means every requested tick completed with a real backend decision for
    /// every human.
    pub error: Option<String>,
}

/// Drive a live-agent run for `config.ticks` ticks.
///
/// Every tick, every human's decision must come from a real backend (hermes,
/// etc.) turn. If any human's backend turn fails, times out, or returns
/// invalid output, the run stops immediately: the offending tick is not
/// committed, and `LiveRunReport::error` carries the reason. This is a
/// deliberate departure from advisory/fallback behavior: the backend is now a
/// required dependency for a live run, not an optional enhancement.
pub async fn run_live(config: LiveRunConfig) -> anyhow::Result<LiveRunReport> {
    let scenario = load_scenario(&config.scenario_path)?;
    let run_id = format!("live-run-{}", scenario.id);
    let mut simulation = Simulation::new(run_id.clone(), scenario.clone());
    simulation.start()?;
    let mut recording = Recording::new(run_id.clone(), &scenario);

    let mut driver = HumanAgentDriver::new();
    let mut backend = backend_impl::backend_session(&scenario, config.timeout_ms)?;

    let mut evidence = Vec::with_capacity(config.ticks as usize);
    let mut run_error: Option<String> = None;

    for _ in 0..config.ticks {
        if simulation.status != RunStatus::Running {
            break;
        }
        let step_result = driver
            .step_with_backend(&mut simulation, &mut backend)
            .await;

        match step_result {
            Ok((step, humans)) => {
                evidence.push(LiveTickEvidence {
                    tick: step.tick,
                    snapshot_hash: step.snapshot_hash.clone(),
                    humans: humans.clone(),
                });
                recording.push(step);
                recording.push_human_turns(humans);
            }
            Err(error) => {
                simulation.fail();
                run_error = Some(error.to_string());
                break;
            }
        }
    }

    let mut evaluation = cockpit_evaluation::evaluate_scenario(&recording, &scenario);
    if let Some(error) = &run_error {
        evaluation = cockpit_evaluation::mark_execution_failed(evaluation, error);
    }
    let evaluation = serde_json::to_value(evaluation)?;

    Ok(LiveRunReport {
        run_id,
        scenario_hash: scenario.scenario_hash,
        ticks: recording.ticks.len(),
        final_snapshot_hash: recording.final_snapshot_hash().map(str::to_string),
        tick_evidence: evidence,
        backend: backend.label(),
        evaluation,
        error: run_error,
    })
}

/// Replay a previously recorded live run deterministically, without any real
/// backend, by feeding the recorded per-human decisions back through the same
/// [`HumanAgentDriver`] via a `RecordedHumanBackend`. Returns the replayed
/// recording, whose final snapshot hash must match the original for a
/// deterministic run.
pub async fn replay_live(
    scenario: cockpit_simulation_core::SimulationScenario,
    source: &Recording,
) -> anyhow::Result<Recording> {
    use cockpit_agent_runtime::RecordedHumanBackend;

    if source.runtime_contract_version != cockpit_recording::CURRENT_RUNTIME_CONTRACT_VERSION {
        anyhow::bail!(
            "live recording runtime contract version {} is incompatible with {}",
            source.runtime_contract_version,
            cockpit_recording::CURRENT_RUNTIME_CONTRACT_VERSION
        );
    }

    let run_id = format!("replay-{}", scenario.id);
    let mut simulation = Simulation::new(run_id.clone(), scenario.clone());
    simulation.start()?;
    let mut recording = Recording::new(run_id, &scenario);
    let mut driver = HumanAgentDriver::new();
    let mut backend = RecordedHumanBackend::from_tick_evidence(&source.human_turns);

    for _ in 0..source.ticks.len() {
        if simulation.status != RunStatus::Running {
            break;
        }
        let (step, humans) = driver
            .step_with_backend(&mut simulation, &mut backend)
            .await
            .map_err(|error| anyhow::anyhow!("live replay diverged: {error}"))?;
        recording.push(step);
        recording.push_human_turns(humans);
    }
    Ok(recording)
}

// The backend session abstraction lets the deterministic default build exercise
// the full per-human driver/recording path without the external iota-core
// process, while the `live-acp` feature swaps in the real ACP backend. Both
// paths honor the mandatory-backend contract identically: `run_live` never
// substitutes a value when a backend call fails, regardless of which backend
// is configured. The synthetic backend is an explicit, always-on stand-in for
// offline/default-build development (documented as such via its `"synthetic"`
// label in every report), not a silent fallback used when a *real* backend
// fails; enabling `live-acp` is what opts a run into calling a real backend at
// all.
#[cfg(not(feature = "live-acp"))]
pub(crate) mod backend_impl {
    use std::collections::BTreeSet;

    use cockpit_agent_runtime::{HumanBackend, HumanTurnContext};
    use cockpit_simulation_core::SimulationScenario;
    use tokio_util::sync::CancellationToken;

    /// Synthetic backend session used when the real ACP backend is not compiled
    /// in. It deterministically returns a valid [`HumanDecision`]-shaped JSON
    /// response for every human so the per-human driver, recording, and replay
    /// path can be exercised end-to-end offline. It is not a fallback for a
    /// failing real backend: it is the entire backend for this build
    /// configuration, and its label (`"synthetic"`) is recorded in every
    /// report so this is never mistaken for a real hermes/ACP call.
    pub struct BackendSession {
        cancellation: CancellationToken,
        handled_alerts: BTreeSet<String>,
    }

    impl HumanBackend for BackendSession {
        async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
            if self.cancellation.is_cancelled() {
                return Err("backend turn cancelled".to_string());
            }
            // Deterministic, persona-flavored stand-in. It responds only to an
            // alert the human is authorized to act on, matching the bounded
            // action contract of a real live backend without accessing ground
            // truth or proposing ungranted commands.
            let action = context
                .observation
                .alerts
                .iter()
                .chain(context.delivered_perception.iter().map(|event| &event.kind))
                .filter(|alert| !self.handled_alerts.contains(*alert))
                .find_map(|alert| action_for_alert(alert).map(|action| (alert.clone(), action)))
                .filter(|(_, action)| {
                    context
                        .action_capabilities
                        .iter()
                        .any(|capability| capability.as_str() == action.2)
                });
            let narrative = if action.is_some() {
                "recognized an actionable cockpit risk and initiated the authorized response"
            } else if context.persona.traits.neuroticism > 0.6 {
                "felt uneasy and watchful"
            } else {
                "monitored the cabin calmly"
            };
            let action_json = action.map_or_else(String::new, |(alert, (target, command, _))| {
                self.handled_alerts.insert(alert);
                format!(r#", "actions": [{{"target": "{target}", "command": "{command}"}}]"#)
            });
            Ok(format!(r#"{{"narrative": "{narrative}"{action_json}}}"#))
        }
    }

    fn action_for_alert(alert: &str) -> Option<(&'static str, &'static str, &'static str)> {
        Some(match alert {
            "SmokeDetected" => ("engine-1", "engineShutdown", "engine.shutdown"),
            "ThermalComfortRisk" => ("hvac-1", "climateComfortRestore", "climate.restoreComfort"),
            "WindshieldVisibilityRisk" => (
                "defogger-1",
                "windshieldDefogActivate",
                "visibility.activateDefog",
            ),
            "DriverFatigueRisk" => (
                "dms-1",
                "fatigueInterventionActivate",
                "driver.activateFatigueIntervention",
            ),
            "ChildPresenceHeatRisk" => (
                "occupant-radar-1",
                "childProtectionActivate",
                "occupant.activateChildProtection",
            ),
            "MedicalEmergencyRisk" => (
                "emergency-call-1",
                "medicalResponseActivate",
                "health.activateMedicalResponse",
            ),
            "MultiUserPrivacyConflict" => (
                "voice-array-1",
                "privacyModeActivate",
                "privacy.activateMode",
            ),
            "EvRangeRisk" => (
                "navigation-1",
                "chargingPlanAccept",
                "energy.acceptChargingPlan",
            ),
            "AdasTakeoverRequired" => (
                "adas-controller-1",
                "adasTakeoverAcknowledge",
                "adas.acknowledgeTakeover",
            ),
            "CyberControlAnomaly" => (
                "security-monitor-1",
                "cyberSafeModeActivate",
                "cybersecurity.enterSafeMode",
            ),
            _ => return None,
        })
    }

    impl BackendSession {
        pub fn label(&self) -> &'static str {
            "synthetic"
        }

        pub async fn warm(&mut self) -> Result<(), String> {
            Ok(())
        }

        pub fn set_turn_cancellation(&mut self, cancellation: CancellationToken) {
            self.cancellation = cancellation;
        }
    }

    pub fn backend_session(
        _scenario: &SimulationScenario,
        _timeout_ms: u64,
    ) -> anyhow::Result<BackendSession> {
        Ok(BackendSession {
            cancellation: CancellationToken::new(),
            handled_alerts: BTreeSet::new(),
        })
    }
}

#[cfg(feature = "live-acp")]
pub(crate) mod backend_impl {
    use cockpit_agent_runtime::{
        HumanBackend, HumanTurnContext,
        acp_adapter::{AcpAdapterConfig, AcpAdapterError, IotaCoreAcpAdapter},
        iota_core_adapter::{CockpitSkill, IotaCoreAdapter},
        live::validate_decision_output,
    };
    use cockpit_simulation_core::SimulationScenario;
    use std::path::Path;
    use tokio_util::sync::CancellationToken;

    fn load_skill(language: &str) -> anyhow::Result<CockpitSkill> {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        IotaCoreAdapter::new(workspace)
            .load_cockpit_skill_localized(language)
            .map_err(anyhow::Error::msg)
    }

    /// Live backend session backed by the real iota-core ACP adapter. Every
    /// human currently shares one adapter/backend selection; per-human backend
    /// selection (e.g. a cheaper model for passengers) can be layered on top
    /// without changing the mandatory-backend contract.
    ///
    /// Retains `adapter_config` so retries can begin with a fresh ACP client
    /// after a stale iota-core execution lock.
    pub struct BackendSession {
        adapter: IotaCoreAcpAdapter,
        adapter_config: AcpAdapterConfig,
        skill: CockpitSkill,
        cancellation: CancellationToken,
    }

    /// How many times to retry a turn that failed solely because iota-core's
    /// persistent execution-lock (see
    /// [`AcpAdapterError::is_stale_execution_lock`]) reports the original
    /// request as still `running`. Follow-up attempts receive an opaque,
    /// unique request marker, so they do not collide with a stale row.
    const STALE_LOCK_MAX_ATTEMPTS: u32 = 3;
    const SESSION_INITIALIZATION_MAX_ATTEMPTS: u32 = 2;
    const SLOW_BACKEND_TURN_LOG_MS: u64 = 1_000;

    impl HumanBackend for BackendSession {
        async fn run_turn(&mut self, context: &HumanTurnContext) -> Result<String, String> {
            let mut last_error = None;
            for attempt in 1..=STALE_LOCK_MAX_ATTEMPTS {
                let turn = if attempt == 1 {
                    self.adapter
                        .execute_cancellable(context, &self.skill, &self.cancellation)
                        .await
                } else {
                    self.adapter
                        .execute_cancellable_after_stale_lock(
                            context,
                            &self.skill,
                            &self.cancellation,
                        )
                        .await
                };
                match turn {
                    Ok(turn) => {
                        let turn = if let Err(reason) = validate_decision_output(&turn.text) {
                            eprintln!(
                                "live backend returned malformed decision output; requesting format retry: human={} backend={} reason={}",
                                context.human_id, turn.backend, reason
                            );
                            self.adapter
                                .execute_cancellable_after_invalid_output(
                                    context,
                                    &self.skill,
                                    &self.cancellation,
                                )
                                .await
                                .map_err(|error| error.to_string())?
                        } else {
                            turn
                        };
                        if turn.elapsed_ms >= SLOW_BACKEND_TURN_LOG_MS {
                            eprintln!(
                                "live backend turn slow: human={} backend={} elapsed_ms={}",
                                context.human_id, turn.backend, turn.elapsed_ms
                            );
                        }
                        return Ok(turn.text);
                    }
                    Err(error) if error.is_session_initialization_failure() => {
                        // `session/new` failed before a prompt was submitted.
                        // Replacing the client is safe and avoids retaining a
                        // Hermes process whose ACP state is already invalid.
                        let mut session_error = error;
                        for session_attempt in 2..=SESSION_INITIALIZATION_MAX_ATTEMPTS {
                            if self.cancellation.is_cancelled() {
                                return Err("backend turn cancelled".to_string());
                            }
                            eprintln!(
                                "live backend session recovery: human={} attempt={}/{} error={}",
                                context.human_id,
                                session_attempt,
                                SESSION_INITIALIZATION_MAX_ATTEMPTS,
                                session_error
                            );
                            self.adapter = IotaCoreAcpAdapter::with_default_config(
                                self.adapter_config.clone(),
                            );
                            if let Err(warm_error) = self.adapter.warm().await {
                                session_error = warm_error;
                                continue;
                            }
                            match self
                                .adapter
                                .execute_cancellable(context, &self.skill, &self.cancellation)
                                .await
                            {
                                Ok(turn) => {
                                    eprintln!(
                                        "live backend turn completed after session recovery: human={} backend={} elapsed_ms={}",
                                        context.human_id, turn.backend, turn.elapsed_ms
                                    );
                                    return Ok(turn.text);
                                }
                                Err(retry_error) => session_error = retry_error,
                            }
                        }
                        eprintln!(
                            "live backend session recovery failed: human={} error={}",
                            context.human_id, session_error
                        );
                        return Err(session_error.to_string());
                    }
                    Err(error) if error.is_stale_execution_lock() => {
                        last_error = Some(error);
                        if attempt < STALE_LOCK_MAX_ATTEMPTS {
                            // Start the retry from a fresh ACP client. The
                            // adapter also adds a fresh opaque marker, which
                            // avoids the stale request-hash row directly.
                            self.adapter = IotaCoreAcpAdapter::with_default_config(
                                self.adapter_config.clone(),
                            );
                        }
                    }
                    Err(error) => {
                        if self.cancellation.is_cancelled() {
                            return Err("backend turn cancelled".to_string());
                        }
                        eprintln!(
                            "live backend turn failed: human={} backend={} error={}",
                            context.human_id,
                            self.label(),
                            error
                        );
                        return Err(error.to_string());
                    }
                }
            }
            let last_error = last_error.unwrap_or_else(|| {
                AcpAdapterError::Turn(
                    "stale-lock retry loop exhausted its attempts without recording a failure; \
                     this indicates a bug in the retry loop rather than a backend error"
                        .to_string(),
                )
            });
            Err(format!(
                "{last_error}. iota-core still rejected all recovery attempts due to an \
                 execution-lock collision. The cockpit retried with independent opaque request \
                 markers, so this is no longer recoverable by clicking Step again; inspect the \
                 upstream ACP/iota-core runtime."
            ))
        }
    }

    impl BackendSession {
        pub fn label(&self) -> &'static str {
            "iota-core-acp"
        }

        pub async fn warm(&mut self) -> Result<(), String> {
            self.adapter
                .warm()
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }

        pub fn set_turn_cancellation(&mut self, cancellation: CancellationToken) {
            self.cancellation = cancellation;
        }
    }

    pub fn backend_session(
        scenario: &SimulationScenario,
        timeout_ms: u64,
    ) -> anyhow::Result<BackendSession> {
        let skill = load_skill(&scenario.language)?;
        let adapter_config = AcpAdapterConfig {
            cwd: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
            timeout_ms,
            ..AcpAdapterConfig::default()
        };
        let adapter = IotaCoreAcpAdapter::with_default_config(adapter_config.clone());
        Ok(BackendSession {
            adapter,
            adapter_config,
            skill,
            cancellation: CancellationToken::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "live-acp"))]
    use super::*;

    /// A backend failure aborts the run immediately: the offending tick is not
    /// committed and the run's error is reported rather than silently
    /// substituted with a rule-based or synthetic decision.
    #[test]
    fn narrativeless_backend_output_is_normalized_by_the_decision_parser() {
        // Narrative prose does not influence simulation behavior, so a
        // backend response without it remains a valid decision with the
        // documented fixed placeholder.
        use cockpit_agent_runtime::live::parse_decision_for_tests as parse_decision;
        let decision =
            parse_decision(r#"{"utterance": "hi"}"#).expect("missing narrative is normalized");
        assert_eq!(decision.narrative, "implicit backend decision");
    }

    #[cfg(not(feature = "live-acp"))]
    #[tokio::test(flavor = "current_thread")]
    async fn live_run_records_per_human_disposition_evidence_per_tick() {
        let report = run_live(LiveRunConfig {
            scenario_path: "../../scenarios/smoke-in-cockpit.yaml".to_string(),
            ticks: 5,
            timeout_ms: 50,
        })
        .await
        .expect("live run completes with the synthetic backend");

        assert!(report.error.is_none(), "no backend failure expected");
        assert!(report.ticks > 0, "at least one tick is committed");
        assert_eq!(
            report.tick_evidence.len(),
            report.ticks,
            "every committed tick carries disposition evidence"
        );
        for tick in &report.tick_evidence {
            assert!(
                !tick.humans.is_empty(),
                "every tick records a decision for at least one human"
            );
        }
        assert!(report.final_snapshot_hash.is_some());
    }
}
