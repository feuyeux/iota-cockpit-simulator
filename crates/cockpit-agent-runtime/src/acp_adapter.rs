use std::path::PathBuf;

use iota_core::{AcpBackend, IotaEngine, acp::AcpPromptOutput, config::NimiaConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    iota_core_adapter::CockpitSkill,
    policy::{AgentRuntimePolicy, FallbackPolicy, TurnDisposition},
};
use cockpit_simulation_core::sensor::Observation;

#[derive(Debug, Clone)]
pub struct AcpAdapterConfig {
    pub backend: String,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
    pub fallback: FallbackPolicy,
}

impl Default for AcpAdapterConfig {
    fn default() -> Self {
        Self {
            backend: "codex".to_string(),
            cwd: PathBuf::from("."),
            timeout_ms: 2_000,
            fallback: FallbackPolicy::RuleAgent,
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
        let disposition = match &turn.disposition {
            TurnDisposition::Completed => "completed".to_string(),
            TurnDisposition::Fallback { policy, reason } => {
                format!("fallback:{policy:?}:{reason}")
            }
        };
        match turn.disposition {
            TurnDisposition::Completed => {
                let output = turn.value;
                let runtime_events = output
                    .events
                    .iter()
                    .filter_map(|event| serde_json::to_value(event).ok())
                    .map(redact_runtime_event)
                    .collect();
                Ok(AcpTurn {
                    backend: self.config.backend.clone(),
                    session_id: output.backend_session_id,
                    text: output.text,
                    runtime_events,
                    elapsed_ms: turn.elapsed_ms,
                    disposition,
                })
            }
            TurnDisposition::Fallback { .. } => {
                let output = turn.value;
                Ok(AcpTurn {
                    backend: self.config.backend.clone(),
                    session_id: None,
                    text: output.text,
                    runtime_events: vec![json!({
                        "kind": "fallback",
                        "backend": self.config.backend,
                        "reason": disposition
                    })],
                    elapsed_ms: turn.elapsed_ms,
                    disposition,
                })
            }
        }
    }
}

fn redact_runtime_event(mut event: Value) -> Value {
    if let Some(object) = event.as_object_mut() {
        for key in ["apiKey", "api_key", "token", "secret", "prompt"] {
            if object.contains_key(key) {
                object.insert(key.to_string(), Value::String("[REDACTED]".to_string()));
            }
        }
    }
    event
}
