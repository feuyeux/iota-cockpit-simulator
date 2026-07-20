use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::Recording;

#[derive(Debug, Clone)]
pub struct PayloadStore {
    root: PathBuf,
}

impl PayloadStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RecordingStoreError> {
        let store = Self { root: root.into() };
        fs::create_dir_all(&store.root)
            .map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        Ok(store)
    }

    pub fn put(&self, payload: &[u8]) -> Result<String, RecordingStoreError> {
        let payload = redact_payload(payload);
        let hash = hash_payload(&payload);
        let path = self.path_for(&hash);
        if path.exists() {
            return Ok(hash);
        }
        fs::create_dir_all(path.parent().expect("payload parent"))
            .map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        let temp = path.with_extension("tmp");
        let mut file =
            fs::File::create(&temp).map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        file.write_all(&payload)
            .map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        file.sync_all()
            .map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        fs::rename(temp, path).map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> Result<Vec<u8>, RecordingStoreError> {
        let bytes = fs::read(self.path_for(hash))
            .map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        if hash_payload(&bytes) != hash {
            return Err(RecordingStoreError::PayloadHashMismatch(hash.to_string()));
        }
        Ok(bytes)
    }

    pub fn path_for(&self, hash: &str) -> PathBuf {
        let digest = hash.strip_prefix("sha256:").unwrap_or(hash);
        self.root
            .join(digest.get(..2).unwrap_or("00"))
            .join(format!("{digest}.json"))
    }
}

fn hash_payload(payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    format!("sha256:{:x}", hasher.finalize())
}

const REDACTED: &str = "[REDACTED]";

fn redact_payload(payload: &[u8]) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(payload) else {
        return payload.to_vec();
    };
    redact_value(&mut value);
    serde_json::to_vec(&value).unwrap_or_else(|_| payload.to_vec())
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(redact_value),
        Value::Object(values) => {
            for (key, value) in values {
                if is_sensitive_key(key) {
                    *value = Value::String(REDACTED.to_string());
                } else {
                    redact_value(value);
                }
            }
        }
        _ => {}
    }
}

fn redact_human_turn_prose(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(redact_human_turn_prose),
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(key.as_str(), "narrative" | "utterance") && value.is_string() {
                    *value = Value::String(REDACTED.to_string());
                } else {
                    redact_human_turn_prose(value);
                }
            }
        }
        _ => {}
    }
}

/// Serialize a Recording for an external process without persisting secrets,
/// prompts, hidden reasoning, narrative, or utterance prose.
pub fn serialize_redacted_recording(recording: &Recording) -> Result<Vec<u8>, RecordingStoreError> {
    let mut value = serde_json::to_value(recording)?;
    redact_human_turn_prose(&mut value);
    redact_value(&mut value);
    Ok(serde_json::to_vec(&value)?)
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey"
            | "token"
            | "authorization"
            | "password"
            | "secret"
            | "credential"
            | "credentials"
            | "prompt"
            | "reasoning"
            | "hiddenreasoning"
            | "chainofthought"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("token")
        || normalized.ends_with("secret")
        || normalized.ends_with("password")
        || normalized.ends_with("credential")
        || normalized.ends_with("credentials")
        || normalized.ends_with("prompt")
}

#[derive(Debug, Error)]
pub enum RecordingStoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("recording serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("recording '{0}' was not found")]
    NotFound(String),
    #[error("recording I/O error: {0}")]
    Io(String),
    #[error("payload hash mismatch: {0}")]
    PayloadHashMismatch(String),
}

pub struct RecordingStore {
    connection: Connection,
    payloads: PayloadStore,
}

impl RecordingStore {
    pub fn open(path: &str) -> Result<Self, RecordingStoreError> {
        let connection = Connection::open(path)?;
        let payloads = PayloadStore::new(Path::new(path).with_extension("payloads"))?;
        let mut store = Self {
            connection,
            payloads,
        };
        store.initialize()?;
        Ok(store)
    }

    /// Open an existing recording store without creating tables, directories,
    /// or files. Independent evaluator processes use this to keep the
    /// simulation recording immutable across the evaluation boundary.
    pub fn open_read_only(path: &str) -> Result<Self, RecordingStoreError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let payloads = PayloadStore {
            root: Path::new(path).with_extension("payloads"),
        };
        Ok(Self {
            connection,
            payloads,
        })
    }
    pub fn in_memory() -> Result<Self, RecordingStoreError> {
        let connection = Connection::open_in_memory()?;
        let payloads = PayloadStore::new(
            std::env::temp_dir().join(format!("cockpit-recording-{}", uuid::Uuid::new_v4())),
        )?;
        let mut store = Self {
            connection,
            payloads,
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn save(&mut self, recording: &Recording) -> Result<(), RecordingStoreError> {
        let mut human_turns_value = serde_json::to_value(&recording.human_turns)?;
        redact_human_turn_prose(&mut human_turns_value);
        redact_value(&mut human_turns_value);
        let human_turns_json = serde_json::to_string(&human_turns_value)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR REPLACE INTO recordings (run_id, schema_version, runtime_contract_version, world_model_version, application_commit, plugin_hashes_json, scenario_id, scenario_hash, seed, clock_json, human_turns_json, provenance_json, open_world_checkpoint_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                recording.run_id,
                recording.schema_version,
                recording.runtime_contract_version,
                recording.world_model_version,
                recording.application_commit,
                serde_json::to_string(&recording.plugin_hashes)?,
                recording.scenario_id,
                recording.scenario_hash,
                recording.seed,
                serde_json::to_string(&recording.clock)?,
                human_turns_json,
                serde_json::to_string(&recording.provenance)?,
                serde_json::to_string(&recording.open_world_checkpoint)?
            ],
        )?;
        transaction.execute(
            "DELETE FROM recording_ticks WHERE run_id = ?1",
            params![recording.run_id],
        )?;
        for tick in &recording.ticks {
            let payload = serde_json::to_vec(tick)?;
            let payload_hash = self.payloads.put(&payload)?;
            transaction.execute(
                "INSERT INTO recording_ticks (run_id, tick, snapshot_hash, payload_hash, payload_size) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    recording.run_id,
                    tick.tick,
                    tick.snapshot_hash,
                    payload_hash,
                    payload.len()
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn load(&self, run_id: &str) -> Result<Recording, RecordingStoreError> {
        let has_checkpoint = self
            .connection
            .prepare("PRAGMA table_info(recordings)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|name| name == "open_world_checkpoint_json");
        let checkpoint_column = if has_checkpoint {
            "open_world_checkpoint_json"
        } else {
            "'null'"
        };
        let metadata_query = format!(
            "SELECT schema_version, scenario_id, scenario_hash, seed, runtime_contract_version, world_model_version, application_commit, plugin_hashes_json, clock_json, human_turns_json, provenance_json, {checkpoint_column} FROM recordings WHERE run_id = ?1"
        );
        let metadata = self
            .connection
            .query_row(&metadata_query, params![run_id], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, u32>(4)?,
                    row.get::<_, u32>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                ))
            })
            .optional()?
            .ok_or_else(|| RecordingStoreError::NotFound(run_id.to_string()))?;

        let mut statement = self.connection.prepare(
            "SELECT payload_hash FROM recording_ticks WHERE run_id = ?1 ORDER BY tick ASC",
        )?;
        let ticks = statement
            .query_map(params![run_id], |row| row.get::<_, String>(0))?
            .map(|payload| -> Result<_, RecordingStoreError> {
                Ok(serde_json::from_slice(&self.payloads.get(&payload?)?)?)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Recording {
            schema_version: metadata.0,
            runtime_contract_version: metadata.4,
            world_model_version: metadata.5,
            application_commit: metadata.6,
            plugin_hashes: serde_json::from_str(&metadata.7)?,
            run_id: run_id.to_string(),
            scenario_id: metadata.1,
            scenario_hash: metadata.2,
            seed: metadata.3,
            clock: serde_json::from_str(&metadata.8)?,
            ticks,
            human_turns: serde_json::from_str(&metadata.9)?,
            provenance: serde_json::from_str(&metadata.10)?,
            open_world_checkpoint: serde_json::from_str(&metadata.11)?,
        })
    }

    fn initialize(&mut self) -> Result<(), RecordingStoreError> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS recordings (
                run_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                runtime_contract_version INTEGER NOT NULL,
                world_model_version INTEGER NOT NULL,
                application_commit TEXT NOT NULL,
                plugin_hashes_json TEXT NOT NULL,
                scenario_id TEXT NOT NULL,
                scenario_hash TEXT NOT NULL,
                seed INTEGER NOT NULL,
                clock_json TEXT NOT NULL,
                human_turns_json TEXT NOT NULL DEFAULT '[]',
                provenance_json TEXT NOT NULL DEFAULT '{}',
                open_world_checkpoint_json TEXT NOT NULL DEFAULT 'null'
             );
             CREATE TABLE IF NOT EXISTS recording_ticks (
                run_id TEXT NOT NULL REFERENCES recordings(run_id) ON DELETE CASCADE,
                tick INTEGER NOT NULL,
                snapshot_hash TEXT NOT NULL,
                payload_hash TEXT NOT NULL,
                payload_size INTEGER NOT NULL,
                PRIMARY KEY(run_id, tick)
             );
             CREATE INDEX IF NOT EXISTS recording_ticks_by_hash
               ON recording_ticks(run_id, snapshot_hash);",
        )?;
        let has_human_turns = self
            .connection
            .prepare("PRAGMA table_info(recordings)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|name| name == "human_turns_json");
        if !has_human_turns {
            self.connection.execute(
                "ALTER TABLE recordings ADD COLUMN human_turns_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        let has_provenance = self
            .connection
            .prepare("PRAGMA table_info(recordings)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|name| name == "provenance_json");
        if !has_provenance {
            self.connection.execute(
                "ALTER TABLE recordings ADD COLUMN provenance_json TEXT NOT NULL DEFAULT '{}'",
                [],
            )?;
        }
        let has_open_world_checkpoint = self
            .connection
            .prepare("PRAGMA table_info(recordings)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|name| name == "open_world_checkpoint_json");
        if !has_open_world_checkpoint {
            self.connection.execute(
                "ALTER TABLE recordings ADD COLUMN open_world_checkpoint_json TEXT NOT NULL DEFAULT 'null'",
                [],
            )?;
        }
        Ok(())
    }
}
