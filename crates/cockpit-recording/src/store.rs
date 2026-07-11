use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::Recording;

#[derive(Debug, Error)]
pub enum RecordingStoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("recording serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("recording '{0}' was not found")]
    NotFound(String),
}

pub struct RecordingStore {
    connection: Connection,
}

impl RecordingStore {
    pub fn open(path: &str) -> Result<Self, RecordingStoreError> {
        let connection = Connection::open(path)?;
        let mut store = Self { connection };
        store.initialize()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self, RecordingStoreError> {
        let connection = Connection::open_in_memory()?;
        let mut store = Self { connection };
        store.initialize()?;
        Ok(store)
    }

    pub fn save(&mut self, recording: &Recording) -> Result<(), RecordingStoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR REPLACE INTO recordings (run_id, schema_version, scenario_id, scenario_hash, seed) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                recording.run_id,
                recording.schema_version,
                recording.scenario_id,
                recording.scenario_hash,
                recording.seed
            ],
        )?;
        transaction.execute(
            "DELETE FROM recording_ticks WHERE run_id = ?1",
            params![recording.run_id],
        )?;
        for tick in &recording.ticks {
            transaction.execute(
                "INSERT INTO recording_ticks (run_id, tick, snapshot_hash, payload_json) VALUES (?1, ?2, ?3, ?4)",
                params![
                    recording.run_id,
                    tick.tick,
                    tick.snapshot_hash,
                    serde_json::to_string(tick)?
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
                "SELECT schema_version, scenario_id, scenario_hash, seed FROM recordings WHERE run_id = ?1",
                params![run_id],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| RecordingStoreError::NotFound(run_id.to_string()))?;

        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM recording_ticks WHERE run_id = ?1 ORDER BY tick ASC",
        )?;
        let ticks = statement
            .query_map(params![run_id], |row| row.get::<_, String>(0))?
            .map(|payload| -> Result<_, RecordingStoreError> {
                Ok(serde_json::from_str(&payload?)?)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Recording {
            schema_version: metadata.0,
            run_id: run_id.to_string(),
            scenario_id: metadata.1,
            scenario_hash: metadata.2,
            seed: metadata.3,
            ticks,
        })
    }

    fn initialize(&mut self) -> Result<(), RecordingStoreError> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS recordings (
                run_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                scenario_id TEXT NOT NULL,
                scenario_hash TEXT NOT NULL,
                seed INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS recording_ticks (
                run_id TEXT NOT NULL REFERENCES recordings(run_id) ON DELETE CASCADE,
                tick INTEGER NOT NULL,
                snapshot_hash TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                PRIMARY KEY(run_id, tick)
             );
             CREATE INDEX IF NOT EXISTS recording_ticks_by_hash
               ON recording_ticks(run_id, snapshot_hash);",
        )?;
        Ok(())
    }
}
