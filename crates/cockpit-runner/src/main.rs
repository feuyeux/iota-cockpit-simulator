use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cockpit-runner")]
#[command(about = "Validate and run deterministic cockpit simulation scenarios")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Bench {
        scenario: PathBuf,
        #[arg(long, default_value_t = 120)]
        ticks: u64,
        #[arg(long, default_value_t = 1000)]
        active_entities: u64,
        #[arg(long, default_value_t = 10000)]
        events_per_minute: u64,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1:47701")]
        bind: String,
        #[arg(long)]
        session_token: String,
        /// Optional SQLite recording database. When set, the served process
        /// persists committed ticks so it can recover after a real restart.
        #[arg(long)]
        recording_db: Option<String>,
    },
    Validate {
        scenario: PathBuf,
    },
    Run {
        scenario: PathBuf,
        #[arg(long, default_value_t = 80)]
        ticks: u64,
    },
    RunLive {
        scenario: PathBuf,
        #[arg(long, default_value_t = 80)]
        ticks: u64,
        #[arg(long, default_value_t = 2_000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 2)]
        max_attempts: usize,
        #[arg(long, default_value_t = 3)]
        circuit_failure_threshold: usize,
    },
    /// Migrate a recording file forward to the current schema version.
    MigrateRecording {
        /// Source recording JSON file.
        input: PathBuf,
        /// Destination for the migrated recording. Defaults to overwriting the
        /// input in place.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Report the migration that would run without writing any output.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Bench {
            scenario,
            ticks,
            active_entities,
            events_per_minute,
        } => {
            let report =
                cockpit_runner::benchmark::run(cockpit_runner::benchmark::BenchmarkConfig {
                    scenario_path: scenario.display().to_string(),
                    ticks,
                    active_entities,
                    events_per_minute,
                })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Serve {
            bind,
            session_token,
            recording_db,
        } => {
            cockpit_runner::server::serve_persistent(&bind, session_token, recording_db.as_deref())
                .await
                .with_context(|| format!("failed to serve runner on {bind}"))?;
        }
        Command::Validate { scenario } => {
            let scenario = cockpit_scenario::load_scenario(&scenario)
                .with_context(|| format!("failed to validate {}", scenario.display()))?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "scenarioId": scenario.id,
                    "scenarioHash": scenario.scenario_hash,
                    "schemaVersion": scenario.schema_version
                })
            );
        }
        Command::Run { scenario, ticks } => {
            let scenario = cockpit_scenario::load_scenario(&scenario)
                .with_context(|| format!("failed to load {}", scenario.display()))?;
            let deadline = scenario.shutdown_deadline_ticks;
            let recording =
                cockpit_recording::run_rule_agent_recording("runner-run-1", scenario, ticks)?;
            let evaluation = cockpit_evaluation::evaluate_smoke_shutdown(&recording, deadline);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "runId": recording.run_id,
                    "scenarioHash": recording.scenario_hash,
                    "ticks": recording.ticks.len(),
                    "finalSnapshotHash": recording.final_snapshot_hash(),
                    "evaluation": evaluation
                }))?
            );
        }
        Command::RunLive {
            scenario,
            ticks,
            timeout_ms,
            max_attempts,
            circuit_failure_threshold,
        } => {
            let report = cockpit_runner::run_live(cockpit_runner::LiveRunConfig {
                scenario_path: scenario.display().to_string(),
                ticks,
                timeout_ms,
                max_attempts,
                circuit_failure_threshold,
            })
            .await
            .with_context(|| format!("failed to run live agent on {}", scenario.display()))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::MigrateRecording {
            input,
            output,
            dry_run,
        } => {
            let bytes = std::fs::read(&input)
                .with_context(|| format!("failed to read {}", input.display()))?;
            let (recording, report) = cockpit_recording::migrate_recording_bytes(&bytes)
                .map_err(|error| anyhow::anyhow!("migration failed: {error}"))?;
            if !dry_run {
                let destination = output.unwrap_or_else(|| input.clone());
                let encoded = serde_json::to_vec_pretty(&recording)?;
                std::fs::write(&destination, encoded)
                    .with_context(|| format!("failed to write {}", destination.display()))?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "fromVersion": report.from_version,
                    "toVersion": report.to_version,
                    "migrated": report.migrated(),
                    "steps": report.steps,
                    "dryRun": dry_run,
                    "runId": recording.run_id,
                    "ticks": recording.ticks.len()
                }))?
            );
        }
    }
    Ok(())
}
