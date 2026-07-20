use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use cockpit_evaluation::plane::{
    EVALUATION_PLANE_SCHEMA_VERSION, EvaluationInput, EvidenceVerdict, HiddenRubric, Verdict,
    schema_hash,
};
use cockpit_recording::{Recording, RecordingStore};
use serde::{Deserialize, Serialize};

const SUITE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteManifest {
    pub schema_version: u32,
    pub suite_id: String,
    pub suite_version: String,
    pub cases: Vec<SuiteCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteCase {
    pub id: String,
    pub rubric: PathBuf,
    #[serde(default)]
    pub scenario: Option<PathBuf>,
    #[serde(default)]
    pub recording: Option<PathBuf>,
    #[serde(default)]
    pub recording_db: Option<PathBuf>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default = "default_ticks")]
    pub ticks: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionMode {
    #[default]
    Deterministic,
    Live,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteCaseReport {
    pub case_id: String,
    pub scenario_id: String,
    pub run_id: String,
    pub report: EvidenceVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infrastructure_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_verdict: Option<Verdict>,
    #[serde(default)]
    pub regressed: bool,
}

impl SuiteCaseReport {
    fn successful(&self) -> bool {
        self.report.verdict == Verdict::Pass && self.report.release_gate_passed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEvaluationReport {
    pub schema_version: u32,
    pub suite_id: String,
    pub suite_version: String,
    pub generated_at_ms: u128,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub inconclusive: usize,
    pub pass_rate: f64,
    pub minimum_pass_rate: f64,
    pub regression_count: usize,
    pub release_gate_passed: bool,
    pub cases: Vec<SuiteCaseReport>,
}

fn default_ticks() -> u64 {
    80
}

fn default_timeout_ms() -> u64 {
    2_000
}

pub fn run_suite<F>(
    manifest_path: &Path,
    simulator_command: &Path,
    minimum_pass_rate: f64,
    baseline_path: Option<&Path>,
    mut evaluate: F,
) -> anyhow::Result<BatchEvaluationReport>
where
    F: FnMut(EvaluationInput, HiddenRubric) -> anyhow::Result<EvidenceVerdict>,
{
    if !(0.0..=1.0).contains(&minimum_pass_rate) {
        anyhow::bail!("minimum pass rate must be in 0..=1");
    }
    let bytes = fs::read(manifest_path)
        .with_context(|| format!("failed to read suite {}", manifest_path.display()))?;
    let manifest: SuiteManifest = serde_yaml::from_slice(&bytes)
        .with_context(|| format!("failed to parse suite {}", manifest_path.display()))?;
    if manifest.schema_version != SUITE_SCHEMA_VERSION {
        anyhow::bail!(
            "suite schema {} is unsupported; expected {}",
            manifest.schema_version,
            SUITE_SCHEMA_VERSION
        );
    }
    if manifest.cases.is_empty() {
        anyhow::bail!("evaluation suite contains no cases");
    }
    let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut ids = std::collections::BTreeSet::new();
    let mut cases = Vec::with_capacity(manifest.cases.len());
    for case in &manifest.cases {
        if case.id.trim().is_empty() || !ids.insert(case.id.clone()) {
            anyhow::bail!("suite case IDs must be non-empty and unique: '{}'", case.id);
        }
        match evaluate_case(case, root, simulator_command, &mut evaluate) {
            Ok(report) => cases.push(report),
            Err(error) => cases.push(infrastructure_failure_case(case, error.to_string())),
        }
    }

    if let Some(path) = baseline_path {
        let baseline: BatchEvaluationReport = serde_json::from_slice(
            &fs::read(path)
                .with_context(|| format!("failed to read baseline {}", path.display()))?,
        )
        .with_context(|| format!("failed to parse baseline {}", path.display()))?;
        if baseline.suite_id != manifest.suite_id {
            anyhow::bail!(
                "baseline suite '{}' does not match '{}'",
                baseline.suite_id,
                manifest.suite_id
            );
        }
        let previous = baseline
            .cases
            .iter()
            .map(|case| (case.case_id.as_str(), case))
            .collect::<BTreeMap<_, _>>();
        for case in &mut cases {
            if let Some(before) = previous.get(case.case_id.as_str()) {
                case.baseline_verdict = Some(before.report.verdict);
                case.regressed = before.successful() && !case.successful();
            }
        }
        let current_ids = cases
            .iter()
            .map(|case| case.case_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let removed = baseline
            .cases
            .iter()
            .filter(|case| !current_ids.contains(&case.case_id))
            .map(missing_baseline_case)
            .collect::<Vec<_>>();
        cases.extend(removed);
    }

    let passed = cases
        .iter()
        .filter(|case| case.report.verdict == Verdict::Pass)
        .count();
    let failed = cases
        .iter()
        .filter(|case| case.report.verdict == Verdict::Fail)
        .count();
    let inconclusive = cases
        .iter()
        .filter(|case| case.report.verdict == Verdict::Inconclusive)
        .count();
    let successful = cases.iter().filter(|case| case.successful()).count();
    let pass_rate = successful as f64 / cases.len() as f64;
    let regression_count = cases.iter().filter(|case| case.regressed).count();
    let release_gate_passed = pass_rate >= minimum_pass_rate && regression_count == 0;

    Ok(BatchEvaluationReport {
        schema_version: SUITE_SCHEMA_VERSION,
        suite_id: manifest.suite_id,
        suite_version: manifest.suite_version,
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        total: cases.len(),
        passed,
        failed,
        inconclusive,
        pass_rate,
        minimum_pass_rate,
        regression_count,
        release_gate_passed,
        cases,
    })
}

fn evaluate_case<F>(
    case: &SuiteCase,
    root: &Path,
    simulator_command: &Path,
    evaluate: &mut F,
) -> anyhow::Result<SuiteCaseReport>
where
    F: FnMut(EvaluationInput, HiddenRubric) -> anyhow::Result<EvidenceVerdict>,
{
    let rubric_path = resolve(root, &case.rubric);
    let rubric_bytes = fs::read(&rubric_path)
        .with_context(|| format!("failed to read private rubric {}", rubric_path.display()))?;
    let rubric: HiddenRubric = serde_yaml::from_slice(&rubric_bytes)
        .with_context(|| format!("failed to parse private rubric {}", rubric_path.display()))?;
    let (recording, execution_error) = load_or_execute(case, root, simulator_command)?;
    let scenario_id = recording.scenario_id.clone();
    let run_id = recording.run_id.clone();
    let mut input = EvaluationInput::new(recording);
    input.execution_error = execution_error;
    Ok(SuiteCaseReport {
        case_id: case.id.clone(),
        scenario_id,
        run_id,
        report: evaluate(input, rubric)?,
        infrastructure_error: None,
        baseline_verdict: None,
        regressed: false,
    })
}

fn infrastructure_verdict(case_id: &str, error: &str) -> EvidenceVerdict {
    EvidenceVerdict {
        schema_version: EVALUATION_PLANE_SCHEMA_VERSION,
        verdict: Verdict::Inconclusive,
        rubric_id: format!("suite-case:{case_id}"),
        rubric_version: "unavailable".to_string(),
        rubric_hash: "unavailable".to_string(),
        input_hash: "unavailable".to_string(),
        schema_hash: schema_hash(),
        deterministic_results: Vec::new(),
        evidence: Vec::new(),
        judges: Vec::new(),
        judge_disagreement: false,
        release_gate_passed: false,
        explanation: format!("evaluation infrastructure failed: {error}"),
    }
}

fn infrastructure_failure_case(case: &SuiteCase, error: String) -> SuiteCaseReport {
    SuiteCaseReport {
        case_id: case.id.clone(),
        scenario_id: case.id.clone(),
        run_id: String::new(),
        report: infrastructure_verdict(&case.id, &error),
        infrastructure_error: Some(error),
        baseline_verdict: None,
        regressed: false,
    }
}

fn missing_baseline_case(before: &SuiteCaseReport) -> SuiteCaseReport {
    let error = "case is missing from the current suite manifest".to_string();
    SuiteCaseReport {
        case_id: before.case_id.clone(),
        scenario_id: before.scenario_id.clone(),
        run_id: before.run_id.clone(),
        report: infrastructure_verdict(&before.case_id, &error),
        infrastructure_error: Some(error),
        baseline_verdict: Some(before.report.verdict),
        regressed: true,
    }
}

fn load_or_execute(
    case: &SuiteCase,
    root: &Path,
    simulator_command: &Path,
) -> anyhow::Result<(Recording, Option<String>)> {
    match (
        case.scenario.as_ref(),
        case.recording.as_ref(),
        case.recording_db.as_ref(),
        case.run_id.as_deref(),
    ) {
        (Some(scenario), None, None, None) => {
            execute_case(case, &resolve(root, scenario), simulator_command)
        }
        (None, Some(recording), None, None) => {
            let path = resolve(root, recording);
            let bytes = fs::read(&path)
                .with_context(|| format!("failed to read recording {}", path.display()))?;
            Ok((serde_json::from_slice(&bytes)?, None))
        }
        (None, None, Some(database), Some(run_id)) => {
            let path = resolve(root, database);
            let store = RecordingStore::open_read_only(
                path.to_str()
                    .ok_or_else(|| anyhow::anyhow!("recording DB path is not UTF-8"))?,
            )?;
            Ok((store.load(run_id)?, None))
        }
        _ => anyhow::bail!(
            "suite case '{}' must define exactly one of scenario, recording, or recordingDb + runId",
            case.id
        ),
    }
}

fn execute_case(
    case: &SuiteCase,
    scenario: &Path,
    simulator_command: &Path,
) -> anyhow::Result<(Recording, Option<String>)> {
    let safe_id = case
        .id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let output_path = std::env::temp_dir().join(format!(
        "cockpit-evaluation-{}-{}-{safe_id}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let subcommand = match case.mode {
        ExecutionMode::Deterministic => "run",
        ExecutionMode::Live => "run-live",
    };
    let mut command = Command::new(simulator_command);
    command
        .arg(subcommand)
        .arg(scenario)
        .arg("--ticks")
        .arg(case.ticks.to_string())
        .arg("--recording-output")
        .arg(&output_path);
    if matches!(case.mode, ExecutionMode::Live) {
        command.arg("--timeout-ms").arg(case.timeout_ms.to_string());
    }
    let output = command.output().with_context(|| {
        format!(
            "failed to launch simulator {} for suite case '{}'",
            simulator_command.display(),
            case.id
        )
    })?;
    let recording_bytes = fs::read(&output_path).with_context(|| {
        format!(
            "simulator did not produce recording for suite case '{}': {}",
            case.id,
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(512)
                .collect::<String>()
        )
    })?;
    let _ = fs::remove_file(&output_path);
    let recording: Recording = serde_json::from_slice(&recording_bytes).with_context(|| {
        format!(
            "simulator produced invalid recording for suite case '{}'",
            case.id
        )
    })?;
    let summary = serde_json::from_slice::<serde_json::Value>(&output.stdout).ok();
    let execution_error = summary
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            (!output.status.success()).then(|| {
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(512)
                    .collect::<String>()
            })
        });
    Ok((recording, execution_error))
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

pub fn write_json_report(path: &Path, report: &BatchEvaluationReport) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(report)?)
        .with_context(|| format!("failed to write JSON report {}", path.display()))
}

pub fn write_junit_report(path: &Path, report: &BatchEvaluationReport) -> anyhow::Result<()> {
    let failures = report
        .cases
        .iter()
        .filter(|case| !case.successful())
        .count();
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\">\n",
        escape_xml(&report.suite_id),
        report.total,
        failures
    );
    for case in &report.cases {
        xml.push_str(&format!(
            "  <testcase classname=\"{}\" name=\"{}\">",
            escape_xml(&report.suite_id),
            escape_xml(&case.case_id)
        ));
        if !case.successful() {
            xml.push_str(&format!(
                "<failure message=\"{:?}\">{}</failure>",
                case.report.verdict,
                escape_xml(&case.report.explanation)
            ));
        }
        if case.regressed {
            xml.push_str("<system-err>regressed from passing baseline</system-err>");
        }
        xml.push_str("</testcase>\n");
    }
    xml.push_str("</testsuite>\n");
    fs::write(path, xml).with_context(|| format!("failed to write JUnit report {}", path.display()))
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionMode, SuiteCase, infrastructure_failure_case, infrastructure_verdict,
        missing_baseline_case,
    };
    use cockpit_evaluation::plane::Verdict;
    use std::path::PathBuf;

    fn suite_case(id: &str) -> SuiteCase {
        SuiteCase {
            id: id.to_string(),
            rubric: PathBuf::from("rubric.yaml"),
            scenario: Some(PathBuf::from("scenario.yaml")),
            recording: None,
            recording_db: None,
            run_id: None,
            mode: ExecutionMode::Deterministic,
            ticks: 1,
            timeout_ms: 1,
        }
    }

    #[test]
    fn infrastructure_failure_is_an_auditable_inconclusive_case() {
        let case =
            infrastructure_failure_case(&suite_case("broken-case"), "simulator failed".into());

        assert_eq!(case.case_id, "broken-case");
        assert_eq!(case.report.verdict, Verdict::Inconclusive);
        assert!(!case.report.release_gate_passed);
        assert_eq!(
            case.infrastructure_error.as_deref(),
            Some("simulator failed")
        );
        assert!(case.report.explanation.contains("simulator failed"));
        assert!(!case.regressed);
    }

    #[test]
    fn missing_passing_baseline_case_is_an_explicit_regression() {
        let mut before = infrastructure_failure_case(&suite_case("removed-case"), String::new());
        before.report = infrastructure_verdict("removed-case", "unused");
        before.report.verdict = Verdict::Pass;
        before.report.release_gate_passed = true;
        before.infrastructure_error = None;

        let missing = missing_baseline_case(&before);

        assert_eq!(missing.case_id, "removed-case");
        assert_eq!(missing.report.verdict, Verdict::Inconclusive);
        assert!(!missing.report.release_gate_passed);
        assert_eq!(missing.baseline_verdict, Some(Verdict::Pass));
        assert!(missing.regressed);
        assert!(
            missing
                .infrastructure_error
                .as_deref()
                .is_some_and(|error| error.contains("missing"))
        );
    }
}
