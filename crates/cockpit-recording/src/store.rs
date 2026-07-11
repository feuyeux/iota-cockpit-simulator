use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension, params};
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
        let hash = hash_payload(payload);
        let path = self.path_for(&hash);
        if path.exists() {
            return Ok(hash);
        }
        fs::create_dir_all(path.parent().expect("payload parent"))
            .map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        let temp = path.with_extension("tmp");
        let mut file =
            fs::File::create(&temp).map_err(|error| RecordingStoreError::Io(error.to_string()))?;
        file.write_all(payload)
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
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR REPLACE INTO recordings (run_id, schema_version, runtime_contract_version, world_model_version, application_commit, plugin_hashes_json, scenario_id, scenario_hash, seed, clock_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                serde_json::to_string(&recording.clock)?
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
        let metadata = self
            .connection
            .query_row(
                "SELECT schema_version, scenario_id, scenario_hash, seed, runtime_contract_version, world_model_version, application_commit, plugin_hashes_json, clock_json FROM recordings WHERE run_id = ?1",
                params![run_id],
                |row| {
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
                    ))
                },
            )
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
                clock_json TEXT NOT NULL
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
        Ok(())
    }
}
