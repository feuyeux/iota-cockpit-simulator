//! Recording schema migration and a compatibility strategy beyond version
//! rejection.
//!
//! Older recordings are migrated forward to the current schema at the
//! deserialization boundary: a legacy recording may be missing fields that the
//! strongly-typed [`Recording`](crate::Recording) requires, so migration
//! operates on a raw JSON `Value` before deserialization. Recordings newer than
//! the current schema, or at an unknown intermediate version with no migration
//! path, are rejected with an explicit error rather than silently accepted.

use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::Recording;

/// Current recording schema version understood by this build.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
/// Current runtime contract version.
pub const CURRENT_RUNTIME_CONTRACT_VERSION: u32 = 1;
/// Current world-model version.
pub const CURRENT_WORLD_MODEL_VERSION: u32 = 1;

/// Outcome of migrating a recording to the current schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub from_version: u32,
    pub to_version: u32,
    /// Human-readable description of each applied migration step.
    pub steps: Vec<String>,
}

impl MigrationReport {
    /// Whether any migration step was applied (the recording was not already
    /// current).
    pub fn migrated(&self) -> bool {
        self.from_version != self.to_version
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error(
        "recording schema version {found} is newer than the supported version {current}; upgrade the application"
    )]
    TooNew { found: u32, current: u32 },
    #[error("recording schema version {found} has no migration path to {current}")]
    Unsupported { found: u32, current: u32 },
    #[error("recording is malformed: {0}")]
    Malformed(String),
}

/// Detect the schema version of a raw recording JSON value. A value with no
/// `schemaVersion` field is treated as legacy version 0 (predates the versioned
/// provenance fields).
pub fn detect_schema_version(value: &Value) -> u32 {
    value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

/// Migrate a raw recording JSON value forward to the current schema.
///
/// Returns the migrated value and a report describing which steps ran. The
/// input is rejected if it is newer than this build or sits at a version with
/// no registered migration path.
pub fn migrate_recording_value(
    mut value: Value,
) -> Result<(Value, MigrationReport), MigrationError> {
    if !value.is_object() {
        return Err(MigrationError::Malformed(
            "recording root is not a JSON object".to_string(),
        ));
    }
    let from_version = detect_schema_version(&value);
    if from_version > CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::TooNew {
            found: from_version,
            current: CURRENT_SCHEMA_VERSION,
        });
    }

    let mut steps = Vec::new();
    let mut current = from_version;
    while current < CURRENT_SCHEMA_VERSION {
        match current {
            0 => {
                migrate_v0_to_v1(object_mut(&mut value)?);
                steps.push(
                    "0->1: added schema/runtime/world-model versions and provenance defaults"
                        .to_string(),
                );
                current = 1;
            }
            other => {
                return Err(MigrationError::Unsupported {
                    found: other,
                    current: CURRENT_SCHEMA_VERSION,
                });
            }
        }
    }

    Ok((
        value,
        MigrationReport {
            from_version,
            to_version: current,
            steps,
        },
    ))
}

/// Migrate raw recording bytes forward and deserialize into a [`Recording`].
pub fn migrate_recording_bytes(
    bytes: &[u8],
) -> Result<(Recording, MigrationReport), MigrationError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| MigrationError::Malformed(error.to_string()))?;
    let (migrated, report) = migrate_recording_value(value)?;
    let recording = serde_json::from_value(migrated)
        .map_err(|error| MigrationError::Malformed(error.to_string()))?;
    Ok((recording, report))
}

/// Fill in the versioned provenance fields introduced in schema version 1 for a
/// legacy (version 0) recording, without overwriting any already-present value.
fn migrate_v0_to_v1(object: &mut Map<String, Value>) {
    object.insert("schemaVersion".to_string(), json!(1));
    fill_default(object, "runtimeContractVersion", json!(1));
    fill_default(object, "worldModelVersion", json!(1));
    fill_default(object, "applicationCommit", json!("unknown"));
    fill_default(object, "pluginHashes", json!([]));
}

fn fill_default(object: &mut Map<String, Value>, key: &str, default: Value) {
    if object.get(key).is_none_or(Value::is_null) {
        object.insert(key.to_string(), default);
    }
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, MigrationError> {
    value
        .as_object_mut()
        .ok_or_else(|| MigrationError::Malformed("recording root is not a JSON object".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_v0() -> Value {
        // A recording that predates the versioned provenance fields: no
        // schemaVersion, runtimeContractVersion, worldModelVersion,
        // applicationCommit, or pluginHashes.
        json!({
            "runId": "legacy-run",
            "scenarioId": "smoke-in-cockpit",
            "scenarioHash": "hash",
            "seed": 42,
            "clock": { "mode": "stepped", "tickMs": 100 },
            "ticks": []
        })
    }

    #[test]
    fn legacy_recording_is_migrated_to_current_schema() {
        let (value, report) = migrate_recording_value(legacy_v0()).expect("migrates");
        assert_eq!(report.from_version, 0);
        assert_eq!(report.to_version, CURRENT_SCHEMA_VERSION);
        assert!(report.migrated());
        assert_eq!(report.steps.len(), 1);
        assert_eq!(value["schemaVersion"], json!(1));
        assert_eq!(value["runtimeContractVersion"], json!(1));
        assert_eq!(value["worldModelVersion"], json!(1));
        assert_eq!(value["applicationCommit"], json!("unknown"));
        assert_eq!(value["pluginHashes"], json!([]));
    }

    #[test]
    fn current_recording_is_a_noop() {
        let mut value = legacy_v0();
        value["schemaVersion"] = json!(1);
        value["runtimeContractVersion"] = json!(1);
        value["worldModelVersion"] = json!(1);
        value["applicationCommit"] = json!("abc123");
        value["pluginHashes"] = json!(["plugin@1:sha256:x"]);
        let (migrated, report) = migrate_recording_value(value.clone()).expect("noop");
        assert!(!report.migrated());
        assert!(report.steps.is_empty());
        // Existing provenance is preserved, not clobbered.
        assert_eq!(migrated["applicationCommit"], json!("abc123"));
        assert_eq!(migrated["pluginHashes"], json!(["plugin@1:sha256:x"]));
    }

    #[test]
    fn newer_recording_is_rejected() {
        let mut value = legacy_v0();
        value["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION + 1);
        let error = migrate_recording_value(value).expect_err("rejected");
        assert_eq!(
            error,
            MigrationError::TooNew {
                found: CURRENT_SCHEMA_VERSION + 1,
                current: CURRENT_SCHEMA_VERSION,
            }
        );
    }

    #[test]
    fn non_object_recording_is_rejected() {
        let error = migrate_recording_value(json!([1, 2, 3])).expect_err("rejected");
        assert!(matches!(error, MigrationError::Malformed(_)));
    }
}
