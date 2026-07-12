use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use cockpit_simulation_core::WorldSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const PLUGIN_API_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginPermission {
    WorldRead,
    WorldWrite,
    Network,
    FilesystemRead,
    ChildProcess,
    Threads,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginFailurePolicy {
    DisablePlugin,
    PauseRun,
    FailRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub api_contract: u32,
    pub permissions: Vec<PluginPermission>,
    pub schema: Value,
    pub hash: String,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDiff {
    pub plugin_id: String,
    pub entity_id: String,
    pub component_path: String,
    pub value: Value,
    pub expected_state_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    Discovered,
    Ready,
    Disabled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginFailure {
    pub plugin_id: String,
    pub version: String,
    pub reason: String,
    pub decision: PluginFailurePolicy,
}

pub trait PluginExecutor: Send {
    fn tick(&mut self, snapshot: &WorldSnapshot) -> Result<Vec<StateDiff>, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginTickOutcome {
    Accepted(Vec<StateDiff>),
    Failed(PluginFailure),
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("manifest parse failed: {0}")]
    ManifestParse(String),
    #[error("manifest field '{0}' is invalid")]
    InvalidField(String),
    #[error("plugin hash mismatch: expected {expected}, actual {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("plugin API contract {actual} is incompatible with {expected}")]
    ApiMismatch { expected: u32, actual: u32 },
    #[error("plugin permission is not allowed: {0:?}")]
    PermissionDenied(PluginPermission),
    #[error("plugin signature is required")]
    SignatureRequired,
    #[error("invalid state diff: {0}")]
    InvalidStateDiff(String),
    #[error("failed to read plugin manifest: {0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct PluginPolicy {
    pub api_contract: u32,
    pub allowed_permissions: BTreeSet<PluginPermission>,
    pub require_signature: bool,
    pub failure_policy: PluginFailurePolicy,
    /// Cooperative per-tick wall-clock budget in milliseconds. A plugin whose
    /// `tick` returns after this budget is treated as a failure and handled by
    /// `failure_policy`. `None` disables the budget.
    ///
    /// This is a cooperative budget: it bounds plugins that return but does not
    /// preempt a hung plugin. OS-level preemption requires out-of-process
    /// execution (see `## 36. 插件生命周期` / ADR in `doc/001.md`).
    pub tick_budget_ms: Option<u64>,
}

impl Default for PluginPolicy {
    fn default() -> Self {
        Self {
            api_contract: PLUGIN_API_VERSION,
            allowed_permissions: [PluginPermission::WorldRead].into_iter().collect(),
            require_signature: false,
            failure_policy: PluginFailurePolicy::DisablePlugin,
            tick_budget_ms: Some(50),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub status: PluginStatus,
}

#[derive(Debug, Default)]
pub struct PluginHost {
    plugins: BTreeMap<String, LoadedPlugin>,
    failures: Vec<PluginFailure>,
}

impl PluginHost {
    pub fn discover(
        &mut self,
        directory: impl AsRef<Path>,
        policy: &PluginPolicy,
    ) -> Vec<PluginFailure> {
        let mut failures = Vec::new();
        let entries = match fs::read_dir(directory.as_ref()) {
            Ok(entries) => entries,
            Err(error) => {
                failures.push(self.failure("<directory>", "unknown", error.to_string(), policy));
                return failures;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yaml" | "yml" | "json")
            ) {
                continue;
            }
            match fs::read(&path)
                .map_err(|error| PluginError::Io(error.to_string()))
                .and_then(|bytes| parse_manifest(&bytes))
                .and_then(|manifest| validate_manifest(manifest, policy))
            {
                Ok(manifest) => {
                    self.plugins.insert(
                        manifest.id.clone(),
                        LoadedPlugin {
                            manifest,
                            status: PluginStatus::Ready,
                        },
                    );
                }
                Err(error) => {
                    let id = path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("unknown");
                    let failure = self.failure(id, "unknown", error.to_string(), policy);
                    self.failures.push(failure.clone());
                    failures.push(failure);
                }
            }
        }
        failures
    }

    pub fn validate_state_diff(
        &self,
        snapshot: &WorldSnapshot,
        diff: &StateDiff,
    ) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get(&diff.plugin_id)
            .ok_or_else(|| PluginError::InvalidStateDiff("plugin is not ready".to_string()))?;
        if !plugin
            .manifest
            .permissions
            .contains(&PluginPermission::WorldWrite)
        {
            return Err(PluginError::PermissionDenied(PluginPermission::WorldWrite));
        }
        if diff.expected_state_version != snapshot.version {
            return Err(PluginError::InvalidStateDiff(
                "state version conflict".to_string(),
            ));
        }
        if !allowed_path(&diff.entity_id, &diff.component_path) {
            return Err(PluginError::InvalidStateDiff(
                "component path is outside plugin write scope".to_string(),
            ));
        }
        validate_value(&diff.component_path, &diff.value)
    }

    pub fn run_tick(
        &mut self,
        plugin_id: &str,
        snapshot: &WorldSnapshot,
        executor: &mut dyn PluginExecutor,
        policy: &PluginPolicy,
    ) -> PluginTickOutcome {
        let Some(plugin) = self.plugins.get(plugin_id) else {
            return PluginTickOutcome::Failed(self.record_failure(
                plugin_id,
                "unknown",
                "plugin is not ready".to_string(),
                policy,
            ));
        };
        let version = plugin.manifest.version.clone();
        if plugin.status != PluginStatus::Ready {
            return PluginTickOutcome::Failed(self.record_failure(
                plugin_id,
                &version,
                "plugin is not ready".to_string(),
                policy,
            ));
        }

        let started = std::time::Instant::now();
        let diffs = match executor.tick(snapshot) {
            Ok(diffs) => diffs,
            Err(reason) => {
                return PluginTickOutcome::Failed(
                    self.record_failure(plugin_id, &version, reason, policy),
                );
            }
        };
        if let Some(budget_ms) = policy.tick_budget_ms {
            let elapsed_ms = started.elapsed().as_millis();
            if elapsed_ms > u128::from(budget_ms) {
                return PluginTickOutcome::Failed(self.record_failure(
                    plugin_id,
                    &version,
                    format!("plugin tick exceeded {budget_ms}ms budget ({elapsed_ms}ms)"),
                    policy,
                ));
            }
        }
        for diff in &diffs {
            if diff.plugin_id != plugin_id {
                return PluginTickOutcome::Failed(self.record_failure(
                    plugin_id,
                    &version,
                    "plugin returned a StateDiff for another plugin".to_string(),
                    policy,
                ));
            }
            if let Err(error) = self.validate_state_diff(snapshot, diff) {
                return PluginTickOutcome::Failed(self.record_failure(
                    plugin_id,
                    &version,
                    error.to_string(),
                    policy,
                ));
            }
        }
        PluginTickOutcome::Accepted(diffs)
    }

    pub fn get(&self, plugin_id: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(plugin_id)
    }

    pub fn manifests(&self) -> impl Iterator<Item = &PluginManifest> {
        self.plugins.values().map(|plugin| &plugin.manifest)
    }

    pub fn plugin_ids(&self) -> impl Iterator<Item = &str> {
        self.plugins.keys().map(String::as_str)
    }

    pub fn failures(&self) -> &[PluginFailure] {
        &self.failures
    }

    fn failure(
        &self,
        plugin_id: &str,
        version: &str,
        reason: String,
        policy: &PluginPolicy,
    ) -> PluginFailure {
        PluginFailure {
            plugin_id: plugin_id.to_string(),
            version: version.to_string(),
            reason,
            decision: policy.failure_policy.clone(),
        }
    }

    fn record_failure(
        &mut self,
        plugin_id: &str,
        version: &str,
        reason: String,
        policy: &PluginPolicy,
    ) -> PluginFailure {
        let failure = self.failure(plugin_id, version, reason, policy);
        if let Some(plugin) = self.plugins.get_mut(plugin_id) {
            plugin.status = match policy.failure_policy {
                PluginFailurePolicy::DisablePlugin => PluginStatus::Disabled,
                PluginFailurePolicy::PauseRun | PluginFailurePolicy::FailRun => {
                    PluginStatus::Failed
                }
            };
        }
        self.failures.push(failure.clone());
        failure
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<PluginManifest, PluginError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| PluginError::ManifestParse(error.to_string()))?;
    if text.trim_start().starts_with('{') {
        serde_json::from_str(text).map_err(|error| PluginError::ManifestParse(error.to_string()))
    } else {
        serde_yaml::from_str(text).map_err(|error| PluginError::ManifestParse(error.to_string()))
    }
}

fn validate_manifest(
    mut manifest: PluginManifest,
    policy: &PluginPolicy,
) -> Result<PluginManifest, PluginError> {
    if manifest.id.trim().is_empty() {
        return Err(PluginError::InvalidField("id".to_string()));
    }
    if manifest.version.trim().is_empty() {
        return Err(PluginError::InvalidField("version".to_string()));
    }
    if manifest.api_contract != policy.api_contract {
        return Err(PluginError::ApiMismatch {
            expected: policy.api_contract,
            actual: manifest.api_contract,
        });
    }
    if policy.require_signature && manifest.signature.as_deref().unwrap_or("").is_empty() {
        return Err(PluginError::SignatureRequired);
    }
    for permission in &manifest.permissions {
        if !policy.allowed_permissions.contains(permission) {
            return Err(PluginError::PermissionDenied(permission.clone()));
        }
    }
    let expected = manifest.hash.clone();
    manifest.hash.clear();
    let canonical = serde_json::to_vec(&manifest)
        .map_err(|error| PluginError::ManifestParse(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    let actual = format!("sha256:{:x}", hasher.finalize());
    if expected != actual {
        return Err(PluginError::HashMismatch { expected, actual });
    }
    manifest.hash = actual;
    Ok(manifest)
}

fn allowed_path(entity_id: &str, component_path: &str) -> bool {
    matches!(
        (entity_id, component_path),
        ("cabin", "environment.smokeDensity")
            | ("cabin", "environment.visibility")
            | ("cabin", "environment.temperatureC")
            | ("pilot-1", "pilot.stress")
            | ("pilot-1", "pilot.attention")
            | ("engine-1", "engine.health")
            | ("alarm-1", "alarm.active")
    )
}

fn validate_value(path: &str, value: &Value) -> Result<(), PluginError> {
    let Some(number) = value.as_f64() else {
        return Err(PluginError::InvalidStateDiff(format!(
            "{path} requires a numeric value"
        )));
    };
    let valid = match path {
        "environment.smokeDensity" => (0.0..=3.0).contains(&number),
        "environment.visibility" | "pilot.stress" | "pilot.attention" | "alarm.active" => {
            (0.0..=1.0).contains(&number)
        }
        "environment.temperatureC" => (-80.0..=100.0).contains(&number),
        "engine.health" => (0.0..=1.0).contains(&number),
        _ => false,
    };
    valid.then_some(()).ok_or_else(|| {
        PluginError::InvalidStateDiff(format!("{path} value is outside its allowed range"))
    })
}
