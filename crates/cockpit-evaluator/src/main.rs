use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::Context;
use clap::Parser;
use cockpit_evaluation::plane::{
    DeterministicEvaluator, DualJudgeEvaluator, EvaluationInput, Evaluator, EvidenceVerdict,
    HiddenRubric, IndependentJudge, JudgeDecision, JudgeRequest, Verdict, schema_hash, stable_hash,
};
use cockpit_recording::Recording;

mod suite;

#[derive(Debug, Parser)]
#[command(name = "cockpit-evaluator")]
#[command(about = "Evaluate immutable cockpit recordings outside the simulation process")]
struct Cli {
    /// Immutable recording JSON produced by the simulator/recording store export.
    #[arg(long)]
    recording: Option<PathBuf>,
    /// Simulator recording SQLite database; requires --run-id.
    #[arg(long)]
    recording_db: Option<PathBuf>,
    /// Run ID to load from --recording-db.
    #[arg(long)]
    run_id: Option<String>,
    /// Private rubric YAML/JSON. This file is never passed to the execution model.
    #[arg(long)]
    rubric: Option<PathBuf>,
    /// Batch suite YAML containing scenario/recording cases and private rubric paths.
    #[arg(long)]
    suite: Option<PathBuf>,
    /// Simulator executable used only for suite cases that define a scenario.
    #[arg(long, default_value = "cockpit-simulator")]
    simulator_command: PathBuf,
    /// Optional JSON batch report output path.
    #[arg(long)]
    json_report: Option<PathBuf>,
    /// Optional JUnit XML batch report output path.
    #[arg(long)]
    junit_report: Option<PathBuf>,
    /// Optional prior batch JSON report used to detect pass-to-non-pass regressions.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Required successful release-gate ratio for a batch.
    #[arg(long, default_value_t = 1.0)]
    minimum_pass_rate: f64,
    /// Optional pre-recorded independent Judge A decision JSON.
    #[arg(long)]
    judge_a: Option<PathBuf>,
    /// Optional pre-recorded independent Judge B decision JSON.
    #[arg(long)]
    judge_b: Option<PathBuf>,
    /// Executable Judge provider A. Receives one JSON request on stdin and
    /// must emit one JudgeDecision JSON object on stdout.
    #[arg(long)]
    judge_a_command: Option<PathBuf>,
    /// Argument passed only to Judge A; repeat for multiple arguments.
    #[arg(long = "judge-a-arg", allow_hyphen_values = true)]
    judge_a_args: Vec<String>,
    /// Executable Judge provider B. Must be a different executable path from A.
    #[arg(long)]
    judge_b_command: Option<PathBuf>,
    /// Argument passed only to Judge B; repeat for multiple arguments.
    #[arg(long = "judge-b-arg", allow_hyphen_values = true)]
    judge_b_args: Vec<String>,
    /// Wall-clock timeout for each isolated Judge provider.
    #[arg(long, default_value_t = 120_000)]
    judge_timeout_ms: u64,
    /// Terminal execution error recorded by an orchestrator, if any.
    #[arg(long)]
    execution_error: Option<String>,
}

struct RecordedJudge {
    decision: JudgeDecision,
}

impl IndependentJudge for RecordedJudge {
    fn judge(
        &self,
        input: &EvaluationInput,
        rubric: &HiddenRubric,
        _deterministic: &EvidenceVerdict,
    ) -> Result<JudgeDecision, String> {
        validate_decision(self.decision.clone(), input, rubric)
    }
}

struct ExternalJudge {
    command: PathBuf,
    args: Vec<String>,
    timeout_ms: u64,
}

impl IndependentJudge for ExternalJudge {
    fn judge(
        &self,
        input: &EvaluationInput,
        rubric: &HiddenRubric,
        deterministic: &EvidenceVerdict,
    ) -> Result<JudgeDecision, String> {
        let payload = serde_json::to_vec(&JudgeRequest {
            input: input.clone(),
            rubric: rubric.clone(),
            deterministic: deterministic.clone(),
        })
        .map_err(|error| format!("judge request serialization failed: {error}"))?;
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                format!(
                    "failed to start Judge provider {}: {error}",
                    self.command.display()
                )
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Judge provider stdout was unavailable".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Judge provider stderr was unavailable".to_string())?;
        let stdout_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(1_048_577)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.take(4_097).read_to_end(&mut bytes).map(|_| bytes)
        });
        child
            .stdin
            .take()
            .ok_or_else(|| "Judge provider stdin was unavailable".to_string())?
            .write_all(&payload)
            .map_err(|error| format!("failed to write Judge request: {error}"))?;

        let started = Instant::now();
        let timeout = Duration::from_millis(self.timeout_ms.max(1));
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("failed to poll Judge provider: {error}"))?
            {
                break status;
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Judge provider {} exceeded {}ms",
                    self.command.display(),
                    self.timeout_ms
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| "Judge provider stdout reader panicked".to_string())?
            .map_err(|error| format!("failed to read Judge stdout: {error}"))?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "Judge provider stderr reader panicked".to_string())?
            .map_err(|error| format!("failed to read Judge stderr: {error}"))?;
        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr);
            return Err(format!(
                "Judge provider {} exited with {}: {}",
                self.command.display(),
                status,
                stderr.chars().take(512).collect::<String>()
            ));
        }
        if stdout.len() > 1_048_576 {
            return Err("Judge provider output exceeds 1 MiB".to_string());
        }
        let decision: JudgeDecision = serde_json::from_slice(&stdout)
            .map_err(|error| format!("Judge provider returned invalid JSON: {error}"))?;
        validate_decision(decision, input, rubric)
    }
}

fn validate_decision(
    decision: JudgeDecision,
    input: &EvaluationInput,
    rubric: &HiddenRubric,
) -> Result<JudgeDecision, String> {
    if decision.provenance.rubric_hash != stable_hash(rubric) {
        return Err("judge rubric hash does not match the private rubric".to_string());
    }
    if decision.provenance.schema_hash != schema_hash() {
        return Err("judge schema hash does not match evaluator output schema".to_string());
    }
    if decision.provenance.judge_id.trim().is_empty()
        || decision.provenance.prompt_hash.trim().is_empty()
        || decision.provenance.model.trim().is_empty()
    {
        return Err("judge identity, model, and prompt hash provenance are required".to_string());
    }
    if !(0.0..=1.0).contains(&decision.confidence) {
        return Err("judge confidence must be in 0..=1".to_string());
    }
    if decision.evidence.is_empty() {
        return Err("judge decision must cite recording evidence".to_string());
    }
    for reference in &decision.evidence {
        if reference.kind.trim().is_empty() {
            return Err("judge evidence kind is required".to_string());
        }
        let Some(tick) = input
            .recording
            .ticks
            .iter()
            .find(|tick| tick.tick == reference.tick)
        else {
            return Err(format!(
                "judge evidence tick {} is absent from the recording",
                reference.tick
            ));
        };
        if let Some(evidence_id) = reference.event_id.as_deref() {
            let event = tick
                .events
                .iter()
                .find(|event| event.event_id == evidence_id);
            let action = tick
                .action_results
                .iter()
                .find(|result| result.request.request_id == evidence_id);
            if event.is_none() && action.is_none() {
                return Err(format!(
                    "judge evidence id '{evidence_id}' is absent from recording tick {}",
                    reference.tick
                ));
            }
            if let Some(entity_id) = reference.entity_id.as_deref() {
                let event_matches = event.is_some_and(|event| {
                    event.payload.target.as_deref() == Some(entity_id) || event.source == entity_id
                });
                let action_matches =
                    action.is_some_and(|result| result.request.target == entity_id);
                if !event_matches && !action_matches {
                    return Err(format!(
                        "judge evidence entity '{entity_id}' does not own evidence id '{evidence_id}'"
                    ));
                }
            }
        }
    }
    Ok(decision)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> anyhow::Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse JSON {}", path.display()))
}

fn evaluate_input(
    cli: &Cli,
    input: &EvaluationInput,
    rubric: &HiddenRubric,
) -> anyhow::Result<EvidenceVerdict> {
    let report = match (
        cli.judge_a.as_ref(),
        cli.judge_b.as_ref(),
        cli.judge_a_command.as_ref(),
        cli.judge_b_command.as_ref(),
    ) {
        (Some(first), Some(second), None, None) => {
            let first = RecordedJudge {
                decision: read_json(first)?,
            };
            let second = RecordedJudge {
                decision: read_json(second)?,
            };
            DualJudgeEvaluator {
                deterministic: DeterministicEvaluator,
                first: &first,
                second: &second,
            }
            .evaluate(input, rubric)
        }
        (None, None, Some(first), Some(second)) => {
            let first_identity = fs::canonicalize(first).with_context(|| {
                format!("failed to resolve Judge A provider {}", first.display())
            })?;
            let second_identity = fs::canonicalize(second).with_context(|| {
                format!("failed to resolve Judge B provider {}", second.display())
            })?;
            if first_identity == second_identity {
                anyhow::bail!("Judge A and Judge B must use different executable paths");
            }
            let first = ExternalJudge {
                command: first_identity,
                args: cli.judge_a_args.clone(),
                timeout_ms: cli.judge_timeout_ms,
            };
            let second = ExternalJudge {
                command: second_identity,
                args: cli.judge_b_args.clone(),
                timeout_ms: cli.judge_timeout_ms,
            };
            DualJudgeEvaluator {
                deterministic: DeterministicEvaluator,
                first: &first,
                second: &second,
            }
            .evaluate(input, rubric)
        }
        (None, None, None, None) => DeterministicEvaluator.evaluate(input, rubric),
        _ => anyhow::bail!(
            "use both --judge-a/--judge-b files or both --judge-a-command/--judge-b-command executables"
        ),
    };
    Ok(report)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if (cli.judge_a_command.is_none() && !cli.judge_a_args.is_empty())
        || (cli.judge_b_command.is_none() && !cli.judge_b_args.is_empty())
    {
        anyhow::bail!("Judge provider arguments require their corresponding command");
    }

    if let Some(suite_path) = cli.suite.as_ref() {
        if cli.recording.is_some()
            || cli.recording_db.is_some()
            || cli.run_id.is_some()
            || cli.rubric.is_some()
            || cli.execution_error.is_some()
        {
            anyhow::bail!("--suite cannot be combined with single-recording inputs");
        }
        if cli.judge_a.is_some() || cli.judge_b.is_some() {
            anyhow::bail!(
                "batch suites require executable Judge providers; pre-recorded decisions are single-input artifacts"
            );
        }
        let report = suite::run_suite(
            suite_path,
            &cli.simulator_command,
            cli.minimum_pass_rate,
            cli.baseline.as_deref(),
            |input, rubric| evaluate_input(&cli, &input, &rubric),
        )?;
        if let Some(path) = cli.json_report.as_deref() {
            suite::write_json_report(path, &report)?;
        }
        if let Some(path) = cli.junit_report.as_deref() {
            suite::write_junit_report(path, &report)?;
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !report.release_gate_passed {
            std::process::exit(2);
        }
        return Ok(());
    }

    if cli.json_report.is_some()
        || cli.junit_report.is_some()
        || cli.baseline.is_some()
        || cli.minimum_pass_rate != 1.0
    {
        anyhow::bail!("batch report and baseline options require --suite");
    }
    let recording: Recording = match (
        cli.recording.as_ref(),
        cli.recording_db.as_ref(),
        cli.run_id.as_deref(),
    ) {
        (Some(path), None, None) => read_json(path)?,
        (None, Some(database), Some(run_id)) => {
            let store = cockpit_recording::RecordingStore::open_read_only(
                database
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("recording DB path is not UTF-8"))?,
            )?;
            store.load(run_id)?
        }
        _ => {
            anyhow::bail!("use either --recording <json> or --recording-db <sqlite> --run-id <id>")
        }
    };
    let rubric_path = cli
        .rubric
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--rubric is required for single-recording evaluation"))?;
    let rubric_bytes = fs::read(rubric_path)
        .with_context(|| format!("failed to read private rubric {}", rubric_path.display()))?;
    let rubric: HiddenRubric = serde_yaml::from_slice(&rubric_bytes)
        .with_context(|| format!("failed to parse private rubric {}", rubric_path.display()))?;
    let mut input = EvaluationInput::new(recording);
    input.execution_error = cli.execution_error.clone();
    let report = evaluate_input(&cli, &input, &rubric)?;

    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.release_gate_passed || report.verdict == Verdict::Fail {
        std::process::exit(2);
    }
    Ok(())
}
