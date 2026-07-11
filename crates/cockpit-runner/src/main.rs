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
    Validate {
        scenario: PathBuf,
    },
    Run {
        scenario: PathBuf,
        #[arg(long, default_value_t = 80)]
        ticks: u64,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
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
    }
    Ok(())
}
