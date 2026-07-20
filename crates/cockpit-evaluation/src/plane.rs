use std::collections::BTreeSet;

use cockpit_recording::Recording;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    EvaluationResult, EvaluationSpec, evaluate_with_policy, is_registered_evaluation_rule,
};

pub const EVALUATION_PLANE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationInput {
    pub schema_version: u32,
    pub recording: Recording,
    #[serde(default)]
    pub execution_error: Option<String>,
}

impl EvaluationInput {
    pub fn new(recording: Recording) -> Self {
        Self {
            schema_version: EVALUATION_PLANE_SCHEMA_VERSION,
            recording,
            execution_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRubric {
    pub rubric_id: String,
    pub version: String,
    pub scenario_id: String,
    #[serde(default)]
    pub scenario_hash: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    pub rules: Vec<EvaluationSpec>,
    #[serde(default)]
    pub gate: EvaluationReleaseGate,
}

fn default_language() -> String {
    "en".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationReleaseGate {
    #[serde(default = "default_true")]
    pub require_deterministic_pass: bool,
    #[serde(default)]
    pub require_two_judges: bool,
    #[serde(default = "default_true")]
    pub require_judge_agreement: bool,
    #[serde(default)]
    pub allow_inconclusive: bool,
}

fn default_true() -> bool {
    true
}

impl Default for EvaluationReleaseGate {
    fn default() -> Self {
        Self {
            require_deterministic_pass: true,
            require_two_judges: false,
            require_judge_agreement: true,
            allow_inconclusive: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReference {
    pub tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeProvenance {
    pub judge_id: String,
    pub model: String,
    pub prompt_hash: String,
    pub rubric_hash: String,
    pub schema_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeDecision {
    pub verdict: Verdict,
    pub confidence: f64,
    pub explanation: String,
    pub evidence: Vec<EvidenceReference>,
    pub provenance: JudgeProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceVerdict {
    pub schema_version: u32,
    pub verdict: Verdict,
    pub rubric_id: String,
    pub rubric_version: String,
    pub rubric_hash: String,
    pub input_hash: String,
    pub schema_hash: String,
    pub deterministic_results: Vec<RuleVerdict>,
    pub evidence: Vec<EvidenceReference>,
    #[serde(default)]
    pub judges: Vec<JudgeDecision>,
    #[serde(default)]
    pub judge_disagreement: bool,
    pub release_gate_passed: bool,
    pub explanation: String,
}

/// Immutable request sent across the process boundary to a concrete model
/// Judge. Providers receive no simulator, Simulator, tool, or mutation handle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeRequest {
    pub input: EvaluationInput,
    pub rubric: HiddenRubric,
    pub deterministic: EvidenceVerdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleVerdict {
    pub rule_id: String,
    pub deadline_tick: u64,
    pub verdict: Verdict,
    pub result: EvaluationResult,
}

pub trait Evaluator {
    fn evaluate(&self, input: &EvaluationInput, rubric: &HiddenRubric) -> EvidenceVerdict;
}

pub trait IndependentJudge {
    fn judge(
        &self,
        input: &EvaluationInput,
        rubric: &HiddenRubric,
        deterministic: &EvidenceVerdict,
    ) -> Result<JudgeDecision, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicEvaluator;

impl Evaluator for DeterministicEvaluator {
    fn evaluate(&self, input: &EvaluationInput, rubric: &HiddenRubric) -> EvidenceVerdict {
        let rubric_hash = stable_hash(rubric);
        let input_hash = stable_hash(input);
        let schema_hash = schema_hash();
        let inconclusive = |explanation: String| EvidenceVerdict {
            schema_version: EVALUATION_PLANE_SCHEMA_VERSION,
            verdict: Verdict::Inconclusive,
            rubric_id: rubric.rubric_id.clone(),
            rubric_version: rubric.version.clone(),
            rubric_hash: rubric_hash.clone(),
            input_hash: input_hash.clone(),
            schema_hash: schema_hash.clone(),
            deterministic_results: Vec::new(),
            evidence: Vec::new(),
            judges: Vec::new(),
            judge_disagreement: false,
            release_gate_passed: rubric.gate.allow_inconclusive,
            explanation,
        };

        if input.schema_version != EVALUATION_PLANE_SCHEMA_VERSION {
            return inconclusive(format!(
                "evaluation input schema {} is unsupported",
                input.schema_version
            ));
        }
        if input.recording.scenario_id != rubric.scenario_id {
            return inconclusive("recording scenario does not match hidden rubric".to_string());
        }
        if rubric
            .scenario_hash
            .as_ref()
            .is_some_and(|hash| hash != &input.recording.scenario_hash)
        {
            return inconclusive(
                "recording scenario hash does not match hidden rubric".to_string(),
            );
        }
        if input.recording.ticks.is_empty() {
            return inconclusive("recording contains no committed ticks".to_string());
        }
        if rubric.rules.is_empty() {
            return inconclusive("hidden rubric contains no rules".to_string());
        }
        if let Some(rule) = rubric
            .rules
            .iter()
            .find(|rule| !is_registered_evaluation_rule(&rule.id))
        {
            return inconclusive(format!(
                "hidden rubric rule '{}' is not registered",
                rule.id
            ));
        }

        let deterministic_results = rubric
            .rules
            .iter()
            .map(|rule| {
                let result = evaluate_with_policy(
                    &input.recording,
                    Some(&rule.id),
                    rule.deadline_tick,
                    &rubric.language,
                    &rule.policy,
                );
                RuleVerdict {
                    rule_id: rule.id.clone(),
                    deadline_tick: rule.deadline_tick,
                    verdict: if result.passed {
                        Verdict::Pass
                    } else {
                        Verdict::Fail
                    },
                    result,
                }
            })
            .collect::<Vec<_>>();
        let mut verdict = if deterministic_results
            .iter()
            .all(|result| result.verdict == Verdict::Pass)
        {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        if input.execution_error.is_some() {
            verdict = Verdict::Fail;
        }
        let evidence = collect_evidence(&input.recording, &deterministic_results);
        let mut report = EvidenceVerdict {
            schema_version: EVALUATION_PLANE_SCHEMA_VERSION,
            verdict,
            rubric_id: rubric.rubric_id.clone(),
            rubric_version: rubric.version.clone(),
            rubric_hash,
            input_hash,
            schema_hash,
            deterministic_results,
            evidence,
            judges: Vec::new(),
            judge_disagreement: false,
            release_gate_passed: false,
            explanation: input
                .execution_error
                .clone()
                .unwrap_or_else(|| match verdict {
                    Verdict::Pass => "all deterministic hidden-rubric gates passed".to_string(),
                    Verdict::Fail => {
                        "one or more deterministic hidden-rubric gates failed".to_string()
                    }
                    Verdict::Inconclusive => {
                        "deterministic evaluation was inconclusive".to_string()
                    }
                }),
        };
        report.release_gate_passed = gate_passes(&report, &rubric.gate);
        report
    }
}

pub struct DualJudgeEvaluator<'a> {
    pub deterministic: DeterministicEvaluator,
    pub first: &'a dyn IndependentJudge,
    pub second: &'a dyn IndependentJudge,
}

impl Evaluator for DualJudgeEvaluator<'_> {
    fn evaluate(&self, input: &EvaluationInput, rubric: &HiddenRubric) -> EvidenceVerdict {
        let mut report = self.deterministic.evaluate(input, rubric);
        if report.verdict == Verdict::Inconclusive {
            return report;
        }
        let first = self.first.judge(input, rubric, &report);
        let second = self.second.judge(input, rubric, &report);
        let (first, second) = match (first, second) {
            (Ok(first), Ok(second)) => (first, second),
            (left, right) => {
                report.verdict = Verdict::Inconclusive;
                report.release_gate_passed = rubric.gate.allow_inconclusive;
                report.explanation = format!(
                    "independent judge unavailable: first={:?}, second={:?}",
                    left.err(),
                    right.err()
                );
                return report;
            }
        };
        if !judges_are_independent(&first, &second) {
            report.verdict = Verdict::Inconclusive;
            report.judges = vec![first, second];
            report.release_gate_passed = rubric.gate.allow_inconclusive;
            report.explanation =
                "independent judges must have distinct judge identities and models".to_string();
            return report;
        }
        report.judge_disagreement = first.verdict != second.verdict;
        let judges_agree_with_deterministic =
            first.verdict == report.verdict && second.verdict == report.verdict;
        report.evidence.extend(first.evidence.iter().cloned());
        report.evidence.extend(second.evidence.iter().cloned());
        report.judges = vec![first, second];
        if report.judge_disagreement || !judges_agree_with_deterministic {
            report.verdict = Verdict::Inconclusive;
            report.explanation =
                "deterministic and independent judge verdicts do not agree".to_string();
        }
        report.release_gate_passed = gate_passes(&report, &rubric.gate);
        report
    }
}

fn judges_are_independent(first: &JudgeDecision, second: &JudgeDecision) -> bool {
    first.provenance.judge_id != second.provenance.judge_id
        && first.provenance.model != second.provenance.model
}

fn gate_passes(report: &EvidenceVerdict, gate: &EvaluationReleaseGate) -> bool {
    if report.verdict == Verdict::Inconclusive && !gate.allow_inconclusive {
        return false;
    }
    if gate.require_deterministic_pass
        && !report
            .deterministic_results
            .iter()
            .all(|result| result.verdict == Verdict::Pass)
    {
        return false;
    }
    if gate.require_two_judges && report.judges.len() != 2 {
        return false;
    }
    if gate.require_judge_agreement && report.judge_disagreement {
        return false;
    }
    true
}

fn collect_evidence(recording: &Recording, results: &[RuleVerdict]) -> Vec<EvidenceReference> {
    let requested_ids = results
        .iter()
        .flat_map(|rule| rule.result.evidence_event_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut evidence = recording
        .ticks
        .iter()
        .flat_map(|tick| tick.events.iter())
        .filter(|event| requested_ids.contains(&event.event_id))
        .map(|event| EvidenceReference {
            tick: event.tick,
            entity_id: event
                .payload
                .target
                .clone()
                .or_else(|| Some(event.source.clone())),
            event_id: Some(event.event_id.clone()),
            kind: event.event_type.clone(),
        })
        .collect::<Vec<_>>();
    for violation in results
        .iter()
        .flat_map(|rule| rule.result.safety_violations.iter())
    {
        evidence.push(EvidenceReference {
            tick: violation.tick,
            entity_id: None,
            event_id: Some(violation.request_id.clone()),
            kind: format!("SafetyViolation:{}", violation.code),
        });
    }
    if evidence.is_empty()
        && let Some(tick) = recording.ticks.last()
    {
        evidence.push(EvidenceReference {
            tick: tick.tick,
            entity_id: None,
            event_id: None,
            kind: "FinalCommittedTick".to_string(),
        });
    }
    evidence
}

pub fn stable_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn schema_hash() -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(b"cockpit-independent-evaluation-plane-schema-v1")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedJudge(JudgeDecision);

    impl IndependentJudge for FixedJudge {
        fn judge(
            &self,
            _input: &EvaluationInput,
            _rubric: &HiddenRubric,
            _deterministic: &EvidenceVerdict,
        ) -> Result<JudgeDecision, String> {
            Ok(self.0.clone())
        }
    }

    fn decision(id: &str, verdict: Verdict) -> JudgeDecision {
        JudgeDecision {
            verdict,
            confidence: 1.0,
            explanation: "fixture".to_string(),
            evidence: Vec::new(),
            provenance: JudgeProvenance {
                judge_id: id.to_string(),
                model: format!("fixture-model-{id}"),
                prompt_hash: "sha256:prompt".to_string(),
                rubric_hash: "sha256:rubric".to_string(),
                schema_hash: schema_hash(),
            },
        }
    }

    #[test]
    fn independent_judges_require_distinct_ids_and_models() {
        let first = decision("a", Verdict::Pass);
        let mut second = decision("b", Verdict::Pass);
        assert!(judges_are_independent(&first, &second));
        second.provenance.model = first.provenance.model.clone();
        assert!(!judges_are_independent(&first, &second));
        second.provenance.model = "another-model".to_string();
        second.provenance.judge_id = first.provenance.judge_id.clone();
        assert!(!judges_are_independent(&first, &second));
    }

    #[test]
    fn gate_rejects_judge_disagreement() {
        let report = EvidenceVerdict {
            schema_version: 1,
            verdict: Verdict::Inconclusive,
            rubric_id: "r".to_string(),
            rubric_version: "1".to_string(),
            rubric_hash: "h".to_string(),
            input_hash: "i".to_string(),
            schema_hash: schema_hash(),
            deterministic_results: Vec::new(),
            evidence: Vec::new(),
            judges: vec![decision("a", Verdict::Pass), decision("b", Verdict::Fail)],
            judge_disagreement: true,
            release_gate_passed: false,
            explanation: String::new(),
        };
        assert!(!gate_passes(&report, &EvaluationReleaseGate::default()));
        let _ = FixedJudge(decision("unused", Verdict::Pass));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn deterministic_plane_emits_hashed_evidence_verdict() {
        let scenario_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios/smoke-in-cockpit.yaml");
        let scenario = cockpit_scenario::load_scenario(&scenario_path).expect("scenario loads");
        let ticks = scenario.max_ticks;
        let recording = cockpit_recording::run_rule_agent_recording(
            "independent-evaluation-run",
            scenario.clone(),
            ticks,
        )
        .expect("recording runs");
        let rubric = HiddenRubric {
            rubric_id: "smoke-private".to_string(),
            version: "1".to_string(),
            scenario_id: scenario.id,
            scenario_hash: Some(scenario.scenario_hash),
            language: scenario.language,
            rules: vec![EvaluationSpec {
                id: "shutdown-before-spread".to_string(),
                deadline_tick: 30,
                policy: crate::EvaluationPolicy::default(),
            }],
            gate: EvaluationReleaseGate::default(),
        };

        let report = DeterministicEvaluator.evaluate(&EvaluationInput::new(recording), &rubric);

        assert_eq!(report.verdict, Verdict::Pass, "{report:#?}");
        assert!(report.release_gate_passed);
        assert!(!report.evidence.is_empty());
        assert!(report.input_hash.starts_with("sha256:"));
        assert!(report.rubric_hash.starts_with("sha256:"));
        assert_eq!(report.schema_hash, schema_hash());
    }
}
