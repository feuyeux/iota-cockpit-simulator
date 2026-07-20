use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use cockpit_evaluation::plane::EvidenceVerdict;
use cockpit_recording::Recording;
use serde::{Deserialize, Serialize};

use crate::simulator_commands::SimulatorState;

const MAX_EVALUATOR_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_HISTORY_REPORTS: usize = 100;

#[derive(Debug, Clone)]
pub struct EvaluationState {
    evaluator_binary: OsString,
    rubric_root: PathBuf,
    history_root: PathBuf,
    default_judges: JudgePairConfig,
}

#[derive(Debug, Clone)]
struct JudgePairConfig {
    judge_a_command: Option<String>,
    judge_a_args: Vec<String>,
    judge_b_command: Option<String>,
    judge_b_args: Vec<String>,
    timeout_ms: u64,
}

impl Default for JudgePairConfig {
    fn default() -> Self {
        Self {
            judge_a_command: None,
            judge_a_args: Vec::new(),
            judge_b_command: None,
            judge_b_args: Vec::new(),
            timeout_ms: default_judge_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationReportRecord {
    pub id: String,
    pub created_at_ms: u64,
    pub run_id: String,
    pub scenario_id: String,
    pub report: EvidenceVerdict,
}

impl EvaluationState {
    pub fn new(
        workspace_root: &Path,
        rubric_root: PathBuf,
        history_root: PathBuf,
    ) -> Result<Self, String> {
        fs::create_dir_all(&history_root).map_err(|error| {
            format!(
                "failed to create evaluation history {}: {error}",
                history_root.display()
            )
        })?;
        Ok(Self {
            evaluator_binary: evaluator_binary(workspace_root),
            rubric_root,
            history_root,
            default_judges: default_judges_from_env()?,
        })
    }

    fn evaluate(
        &self,
        recording: Recording,
        scenario_id: &str,
        judges: JudgePairConfig,
    ) -> Result<EvaluationReportRecord, String> {
        validate_identifier(scenario_id)?;
        let rubric = self.rubric_root.join(format!("{scenario_id}.yaml"));
        if !rubric.is_file() {
            return Err(format!(
                "private rubric for scenario '{scenario_id}' was not found"
            ));
        }
        let created_at_ms = now_ms();
        let safe_run_id = safe_file_component(&recording.run_id);
        let input_path = self.history_root.join(format!(
            ".evaluation-input-{safe_run_id}-{created_at_ms}.json"
        ));
        fs::write(
            &input_path,
            cockpit_recording::serialize_redacted_recording(&recording)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("failed to write evaluator input: {error}"))?;

        let result = self.invoke_evaluator(&input_path, &rubric, &judges);
        let _ = fs::remove_file(&input_path);
        let report = result?;
        let record = EvaluationReportRecord {
            id: format!("{created_at_ms}-{safe_run_id}"),
            created_at_ms,
            run_id: recording.run_id,
            scenario_id: scenario_id.to_string(),
            report,
        };
        self.persist(&record)?;
        Ok(record)
    }

    fn invoke_evaluator(
        &self,
        input_path: &Path,
        rubric: &Path,
        judges: &JudgePairConfig,
    ) -> Result<EvidenceVerdict, String> {
        let configured_a = judges.judge_a_command.as_deref();
        let configured_b = judges.judge_b_command.as_deref();
        if configured_a.is_some() != configured_b.is_some() {
            return Err("both Judge provider commands are required".to_string());
        }
        if configured_a.is_none()
            && (!judges.judge_a_args.is_empty() || !judges.judge_b_args.is_empty())
        {
            return Err("Judge arguments require provider commands".to_string());
        }

        let mut command = Command::new(&self.evaluator_binary);
        command
            .arg("--recording")
            .arg(input_path)
            .arg("--rubric")
            .arg(rubric)
            .arg("--judge-timeout-ms")
            .arg(judges.timeout_ms.max(1).to_string());
        if let (Some(first), Some(second)) = (configured_a, configured_b) {
            command.arg("--judge-a-command").arg(first);
            for argument in &judges.judge_a_args {
                command.arg("--judge-a-arg").arg(argument);
            }
            command.arg("--judge-b-command").arg(second);
            for argument in &judges.judge_b_args {
                command.arg("--judge-b-arg").arg(argument);
            }
        }
        let output = command.output().map_err(|error| {
            format!(
                "failed to start independent evaluator {:?}: {error}",
                self.evaluator_binary
            )
        })?;
        if output.stdout.len() > MAX_EVALUATOR_OUTPUT_BYTES {
            return Err("independent evaluator output exceeds 2 MiB".to_string());
        }
        match serde_json::from_slice::<EvidenceVerdict>(&output.stdout) {
            Ok(report) => Ok(report),
            Err(error) => Err(format!(
                "independent evaluator failed ({}): {}; {}",
                output.status,
                error,
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(512)
                    .collect::<String>()
            )),
        }
    }

    fn persist(&self, record: &EvaluationReportRecord) -> Result<(), String> {
        let path = self.history_root.join(format!("{}.json", record.id));
        let temporary = path.with_extension("tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("failed to write evaluation history: {error}"))?;
        fs::rename(&temporary, &path)
            .map_err(|error| format!("failed to commit evaluation history: {error}"))?;
        self.prune_history()
    }

    fn prune_history(&self) -> Result<(), String> {
        let mut paths = fs::read_dir(&self.history_root)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
        for path in paths.into_iter().skip(MAX_HISTORY_REPORTS) {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to prune evaluation history {}: {error}",
                    path.display()
                )
            })?;
        }
        Ok(())
    }

    fn history(&self) -> Result<Vec<EvaluationReportRecord>, String> {
        let entries = fs::read_dir(&self.history_root)
            .map_err(|error| format!("failed to read evaluation history: {error}"))?;
        let mut reports = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            if let Ok(report) = serde_json::from_slice::<EvaluationReportRecord>(&bytes) {
                reports.push(report);
            }
        }
        reports.sort_by_key(|report| std::cmp::Reverse(report.created_at_ms));
        reports.truncate(MAX_HISTORY_REPORTS);
        Ok(reports)
    }
}

#[tauri::command]
pub async fn evaluate_run(
    simulator: tauri::State<'_, SimulatorState>,
    evaluator: tauri::State<'_, EvaluationState>,
    run_id: String,
    scenario_id: String,
) -> Result<EvaluationReportRecord, String> {
    let recording = simulator.recording_snapshot(&run_id)?;
    if recording.scenario_id != scenario_id {
        return Err("run scenario does not match the requested rubric".to_string());
    }
    let state = evaluator.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let judges = state.default_judges.clone();
        state.evaluate(recording, &scenario_id, judges)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn list_evaluation_reports(
    evaluator: tauri::State<'_, EvaluationState>,
) -> Result<Vec<EvaluationReportRecord>, String> {
    evaluator.history()
}

fn evaluator_binary(workspace_root: &Path) -> OsString {
    if let Some(binary) = std::env::var_os("COCKPIT_EVALUATOR_BIN") {
        return binary;
    }
    if !cfg!(debug_assertions)
        && let Ok(current_exe) = std::env::current_exe()
        && let Some(path) = bundled_evaluator_path(&current_exe)
        && path.is_file()
    {
        return path.into_os_string();
    }
    let debug_binary = workspace_root
        .join("target")
        .join("debug")
        .join(if cfg!(windows) {
            "cockpit-evaluator.exe"
        } else {
            "cockpit-evaluator"
        });
    if debug_binary.is_file() {
        return debug_binary.into_os_string();
    }
    OsString::from(if cfg!(windows) {
        "cockpit-evaluator.exe"
    } else {
        "cockpit-evaluator"
    })
}

fn bundled_evaluator_path(current_exe: &Path) -> Option<PathBuf> {
    let executable_dir = current_exe.parent()?;
    let base_dir = if executable_dir.ends_with("deps") {
        executable_dir.parent().unwrap_or(executable_dir)
    } else {
        executable_dir
    };
    Some(base_dir.join(if cfg!(windows) {
        "cockpit-evaluator.exe"
    } else {
        "cockpit-evaluator"
    }))
}

fn validate_identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("scenario ID contains unsupported characters".to_string());
    }
    Ok(())
}

fn safe_file_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(96)
        .collect()
}

fn default_judges_from_env() -> Result<JudgePairConfig, String> {
    let first = std::env::var_os("COCKPIT_JUDGE_A_BIN")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "COCKPIT_JUDGE_A_BIN is not UTF-8".to_string())
        })
        .transpose()?;
    let second = std::env::var_os("COCKPIT_JUDGE_B_BIN")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "COCKPIT_JUDGE_B_BIN is not UTF-8".to_string())
        })
        .transpose()?;
    if first.is_some() != second.is_some() {
        return Err(
            "COCKPIT_JUDGE_A_BIN and COCKPIT_JUDGE_B_BIN must be configured together".to_string(),
        );
    }
    let parse_args = |name: &str| -> Result<Vec<String>, String> {
        let Some(value) = std::env::var_os(name) else {
            return Ok(Vec::new());
        };
        let value = value
            .into_string()
            .map_err(|_| format!("{name} is not UTF-8"))?;
        serde_json::from_str(&value)
            .map_err(|error| format!("{name} must be a JSON string array: {error}"))
    };
    let timeout_ms = std::env::var("COCKPIT_JUDGE_TIMEOUT_MS")
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| format!("COCKPIT_JUDGE_TIMEOUT_MS is invalid: {error}"))
        })
        .transpose()?
        .unwrap_or_else(default_judge_timeout_ms);
    Ok(JudgePairConfig {
        judge_a_command: first,
        judge_a_args: parse_args("COCKPIT_JUDGE_A_ARGS_JSON")?,
        judge_b_command: second,
        judge_b_args: parse_args("COCKPIT_JUDGE_B_ARGS_JSON")?,
        timeout_ms,
    })
}

fn default_judge_timeout_ms() -> u64 {
    120_000
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_identifier_cannot_escape_the_private_rubric_root() {
        assert!(validate_identifier("smoke-in-cockpit").is_ok());
        assert!(validate_identifier("../private").is_err());
        assert!(validate_identifier("a/b").is_err());
    }

    #[test]
    fn bundled_evaluator_is_resolved_next_to_the_desktop_executable() {
        let executable = Path::new("target/release/cockpit-desktop");
        let expected = Path::new("target/release").join(if cfg!(windows) {
            "cockpit-evaluator.exe"
        } else {
            "cockpit-evaluator"
        });
        assert_eq!(bundled_evaluator_path(executable), Some(expected));
    }
}
