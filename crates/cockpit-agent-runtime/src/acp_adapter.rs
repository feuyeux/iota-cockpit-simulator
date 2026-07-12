use std::path::PathBuf;

use iota_core::{AcpBackend, IotaEngine, acp::AcpPromptOutput, config::NimiaConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    iota_core_adapter::CockpitSkill,
    policy::{AgentRuntimePolicy, FallbackPolicy, TurnDisposition},
    redact_json,
};
use cockpit_simulation_core::sensor::Observation;

#[derive(Debug, Clone)]
pub struct AcpAdapterConfig {
    pub backend: String,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
    pub fallback: FallbackPolicy,
    pub max_attempts: usize,
    pub circuit_failure_threshold: usize,
}

impl Default for AcpAdapterConfig {
    fn default() -> Self {
        Self {
            backend: "codex".to_string(),
            cwd: PathBuf::from("."),
            timeout_ms: 2_000,
            fallback: FallbackPolicy::RuleAgent,
            max_attempts: 2,
            circuit_failure_threshold: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpTurn {
    pub backend: String,
    pub session_id: Option<String>,
    pub text: String,
    pub runtime_events: Vec<Value>,
    pub elapsed_ms: u64,
    pub disposition: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AcpAdapterError {
    #[error("invalid ACP backend: {0}")]
    InvalidBackend(String),
    #[error("ACP turn failed: {0}")]
    Turn(String),
}

pub struct IotaCoreAcpAdapter {
    engine: IotaEngine,
    config: AcpAdapterConfig,
    policy: AgentRuntimePolicy,
}

impl IotaCoreAcpAdapter {
    pub fn new(config: NimiaConfig, adapter_config: AcpAdapterConfig) -> Self {
        let policy = AgentRuntimePolicy::new(
            adapter_config.timeout_ms,
            1,
            adapter_config.fallback.clone(),
        )
        .with_retry(
            adapter_config.max_attempts,
            adapter_config.circuit_failure_threshold,
        );
        Self {
            engine: IotaEngine::create_session(
                config,
                false,
                adapter_config.timeout_ms,
                Some(&adapter_config.cwd),
            ),
            config: adapter_config,
            policy,
        }
    }

    pub fn build_prompt(observation: &Observation, skill: &CockpitSkill) -> String {
        let perceived_world =
            serde_json::to_string(observation).unwrap_or_else(|_| "{}".to_string());
        format!(
            "You are the cockpit simulation agent.\n\nSkill:\n{}\n\nAuthorized perceived observation JSON:\n{}\n\nReturn a concise decision and use only the authorized simulation tools. Never invent Ground Truth fields.",
            skill.body, perceived_world
        )
    }

    pub async fn execute(
        &mut self,
        observation: &Observation,
        skill: &CockpitSkill,
        fallback_text: impl FnOnce() -> String,
    ) -> Result<AcpTurn, AcpAdapterError> {
        let backend = AcpBackend::parse(&self.config.backend)
            .map_err(|error| AcpAdapterError::InvalidBackend(error.to_string()))?;
        let prompt = Self::build_prompt(observation, skill);
        let cwd = self.config.cwd.clone();
        let future = async {
            self.engine
                .run_with_timing(backend, cwd, &prompt)
                .await
                .map_err(|error| error.to_string())
        };
        let turn = self
            .policy
            .execute(future, || AcpPromptOutput::synthetic(fallback_text()))
            .await;
        Ok(self.shape_turn(turn))
    }

    /// Run a turn that can be cancelled mid-flight via `cancel`.
    ///
    /// When the token fires, iota-core's `run_cancellable` tells the live ACP
    /// process to stop and returns `TurnCancelled`; the policy records a
    /// [`TurnDisposition::Cancelled`] and the resulting [`AcpTurn`] carries a
    /// `cancelled:...` disposition for durable evidence.
    ///
    /// Note: Unlike `execute`, cancellable operations don't support retry since
    /// cancellation is an intentional stop, not a transient failure.
    pub async fn execute_cancellable(
        &mut self,
        observation: &Observation,
        skill: &CockpitSkill,
        cancel: &CancellationToken,
        fallback_text: impl FnOnce() -> String,
    ) -> Result<AcpTurn, AcpAdapterError> {
        let backend = AcpBackend::parse(&self.config.backend)
            .map_err(|error| AcpAdapterError::InvalidBackend(error.to_string()))?;
        let prompt = Self::build_prompt(observation, skill);
        let cwd = self.config.cwd.clone();
        
        // Build the cancellable operation as a single future
        let operation = async {
            self.engine
                .run_cancellable(backend, cwd, &prompt, None, Some(cancel))
                .await
                .map_err(|error| {
                    let err_str = error.to_string();
                    // Tag cancellation errors for policy detection
                    if err_str.contains("TurnCancelled") || err_str.contains("cancelled") {
                        format!("__CANCELLED__:{}", err_str)
                    } else {
                        err_str
                    }
                })
        };
        
        // Use execute_cancellable_once which doesn't retry
        let turn = self
            .policy
            .execute_cancellable_once(
                operation,
                cancel,
                || AcpPromptOutput::synthetic(fallback_text()),
            )
            .await;
        Ok(self.shape_turn(turn))
    }

    /// Convert a policy [`AgentTurn`] into the redacted, evidence-carrying
    /// [`AcpTurn`] returned to callers.
    fn shape_turn(&self, turn: crate::policy::AgentTurn<AcpPromptOutput>) -> AcpTurn {
        let disposition = match &turn.disposition {
            TurnDisposition::Completed => "completed".to_string(),
            TurnDisposition::Fallback { policy, reason } => {
                format!("fallback:{policy:?}:{reason}")
            }
            TurnDisposition::Cancelled { reason } => format!("cancelled:{reason}"),
        };
        match turn.disposition {
            TurnDisposition::Completed => {
                let output = turn.value;
                let runtime_events = output
                    .events
                    .iter()
                    .filter_map(|event| serde_json::to_value(event).ok())
                    .map(redact_json)
                    .collect();
                AcpTurn {
                    backend: self.config.backend.clone(),
                    session_id: output.backend_session_id,
                    text: output.text,
                    runtime_events,
                    elapsed_ms: turn.elapsed_ms,
                    disposition,
                }
            }
            TurnDisposition::Fallback { .. } | TurnDisposition::Cancelled { .. } => {
                let output = turn.value;
                let kind = if matches!(turn.disposition, TurnDisposition::Cancelled { .. }) {
                    "cancelled"
                } else {
                    "fallback"
                };
                AcpTurn {
                    backend: self.config.backend.clone(),
                    session_id: None,
                    text: output.text,
                    runtime_events: vec![json!({
                        "kind": kind,
                        "backend": self.config.backend,
                        "reason": disposition
                    })],
                    elapsed_ms: turn.elapsed_ms,
                    disposition,
                }
            }
        }
    }
}
