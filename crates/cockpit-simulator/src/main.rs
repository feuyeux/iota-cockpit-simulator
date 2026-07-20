use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cockpit-simulator")]
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
    #[command(hide = true)]
    McpBridge {
        #[arg(long)]
        state: PathBuf,
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
        /// Write the complete immutable Recording JSON for an external evaluator.
        #[arg(long)]
        recording_output: Option<PathBuf>,
    },
    RunLive {
        scenario: PathBuf,
        #[arg(long, default_value_t = 80)]
        ticks: u64,
        #[arg(long, default_value_t = 2_000)]
        timeout_ms: u64,
        /// Write the complete immutable Recording JSON for an external evaluator.
        #[arg(long)]
        recording_output: Option<PathBuf>,
    },
}

fn write_recording(
    path: Option<PathBuf>,
    recording: &cockpit_recording::Recording,
) -> anyhow::Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let bytes = cockpit_recording::serialize_redacted_recording(recording)?;
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write recording {}", path.display()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Keep iota-core's structured ACP phase logs visible in the simulator sidecar
    // stderr. The guard must live until process exit so exporters can flush.
    let _telemetry_guard =
        iota_core::telemetry::init(&iota_core::telemetry::TelemetryConfig::default())?;
    let cli = Cli::parse();
    match cli.command {
        Command::McpBridge { state } => {
            cockpit_agent::native_mcp::run_stdio(state).map_err(anyhow::Error::msg)?;
        }
        Command::Bench {
            scenario,
            ticks,
            active_entities,
            events_per_minute,
        } => {
            let report =
                cockpit_simulator::benchmark::run(cockpit_simulator::benchmark::BenchmarkConfig {
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
            cockpit_simulator::server::serve_persistent(
                &bind,
                session_token,
                recording_db.as_deref(),
            )
            .await
            .with_context(|| format!("failed to serve simulator on {bind}"))?;
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
        Command::Run {
            scenario,
            ticks,
            recording_output,
        } => {
            let scenario = cockpit_scenario::load_scenario(&scenario)
                .with_context(|| format!("failed to load {}", scenario.display()))?;
            let recording = cockpit_recording::run_rule_agent_recording(
                "simulator-run-1",
                scenario.clone(),
                ticks,
            )?;
            write_recording(recording_output, &recording)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "runId": recording.run_id,
                    "scenarioHash": recording.scenario_hash,
                    "ticks": recording.ticks.len(),
                    "finalSnapshotHash": recording.final_snapshot_hash(),
                    "evaluation": {
                        "status": "pending",
                        "evaluator": "cockpit-evaluator"
                    }
                }))?
            );
        }
        Command::RunLive {
            scenario,
            ticks,
            timeout_ms,
            recording_output,
        } => {
            let report = cockpit_simulator::run_live(cockpit_simulator::LiveRunConfig {
                scenario_path: scenario.display().to_string(),
                ticks,
                timeout_ms,
            })
            .await
            .with_context(|| format!("failed to run live agent on {}", scenario.display()))?;
            write_recording(recording_output, &report.recording)?;
            let run_failed = report.error.is_some();
            println!("{}", serde_json::to_string_pretty(&report)?);
            if run_failed {
                anyhow::bail!(
                    "live run aborted by a mandatory backend failure: {}",
                    report.error.unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    const BUNDLED_SCENARIOS: &[&str] = &[
        "scenarios/smoke-in-cockpit.yaml",
        "scenarios/heatwave-thermal-comfort.yaml",
        "scenarios/winter-defog-visibility.yaml",
        "scenarios/driver-fatigue-guardian.yaml",
        "scenarios/child-left-behind.yaml",
        "scenarios/medical-emergency.yaml",
        "scenarios/voice-privacy-conflict.yaml",
        "scenarios/ev-range-anxiety.yaml",
        "scenarios/adas-takeover-construction.yaml",
        "scenarios/cybersecurity-anomalous-control.yaml",
    ];

    #[test]
    fn every_bundled_public_scenario_runs_without_embedded_scoring() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative_path in BUNDLED_SCENARIOS {
            let path = workspace_root.join(relative_path);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(!source.contains("evaluation:"), "{}", path.display());
            assert!(!source.contains("deadlineTick"), "{}", path.display());
            let scenario = cockpit_scenario::load_scenario(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(!scenario.public_goals.is_empty(), "{}", path.display());
            cockpit_recording::run_rule_agent_recording(
                format!("simulator-public-{}", scenario.id),
                scenario.clone(),
                scenario.max_ticks,
            )
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
    }
}
