use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Recording;

const MAX_TICK_DIFFERENCES: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDiff {
    pub equivalent: bool,
    pub source_final_snapshot_hash: Option<String>,
    pub candidate_final_snapshot_hash: Option<String>,
    pub source_metrics: RecordingMetrics,
    pub candidate_metrics: RecordingMetrics,
    pub first_divergence: Option<TickDiff>,
    pub tick_differences: Vec<TickDiff>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingMetrics {
    pub ticks: usize,
    pub events: usize,
    pub tool_calls: usize,
    pub action_results: usize,
    pub state_diffs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickDiff {
    pub tick: u64,
    pub source_snapshot_hash: Option<String>,
    pub candidate_snapshot_hash: Option<String>,
    pub events_match: bool,
    pub tool_calls_match: bool,
    pub action_results_match: bool,
    pub state_diffs_match: bool,
}

pub fn diff_recordings(source: &Recording, candidate: &Recording) -> RecordingDiff {
    let mut differences = Vec::new();
    let max_ticks = source.ticks.len().max(candidate.ticks.len());
    let mut truncated = false;

    for index in 0..max_ticks {
        let source_tick = source.ticks.get(index);
        let candidate_tick = candidate.ticks.get(index);
        let difference = TickDiff {
            tick: source_tick
                .map(|tick| tick.tick)
                .or_else(|| candidate_tick.map(|tick| tick.tick))
                .unwrap_or(index as u64),
            source_snapshot_hash: source_tick.map(|tick| tick.snapshot_hash.clone()),
            candidate_snapshot_hash: candidate_tick.map(|tick| tick.snapshot_hash.clone()),
            events_match: normalized_field_matches(source_tick, candidate_tick, "events"),
            tool_calls_match: normalized_field_matches(source_tick, candidate_tick, "toolCalls"),
            action_results_match: normalized_field_matches(
                source_tick,
                candidate_tick,
                "actionResults",
            ),
            state_diffs_match: normalized_field_matches(source_tick, candidate_tick, "stateDiffs"),
        };
        let snapshot_matches =
            difference.source_snapshot_hash == difference.candidate_snapshot_hash;
        if snapshot_matches
            && difference.events_match
            && difference.tool_calls_match
            && difference.action_results_match
            && difference.state_diffs_match
        {
            continue;
        }
        if differences.len() == MAX_TICK_DIFFERENCES {
            truncated = true;
            break;
        }
        differences.push(difference);
    }

    RecordingDiff {
        equivalent: differences.is_empty() && source.ticks.len() == candidate.ticks.len(),
        source_final_snapshot_hash: source.final_snapshot_hash().map(ToString::to_string),
        candidate_final_snapshot_hash: candidate.final_snapshot_hash().map(ToString::to_string),
        source_metrics: metrics(source),
        candidate_metrics: metrics(candidate),
        first_divergence: differences.first().cloned(),
        tick_differences: differences,
        truncated,
    }
}

fn metrics(recording: &Recording) -> RecordingMetrics {
    RecordingMetrics {
        ticks: recording.ticks.len(),
        events: recording.ticks.iter().map(|tick| tick.events.len()).sum(),
        tool_calls: recording
            .ticks
            .iter()
            .map(|tick| tick.tool_calls.len())
            .sum(),
        action_results: recording
            .ticks
            .iter()
            .map(|tick| tick.action_results.len())
            .sum(),
        state_diffs: recording
            .ticks
            .iter()
            .map(|tick| tick.state_diffs.len())
            .sum(),
    }
}

fn normalized_field_matches(
    source: Option<&cockpit_world::StepRecord>,
    candidate: Option<&cockpit_world::StepRecord>,
    field: &str,
) -> bool {
    let Some(source) = source else {
        return candidate.is_none();
    };
    let Some(candidate) = candidate else {
        return false;
    };
    let source = serde_json::to_value(source).unwrap_or(Value::Null);
    let candidate = serde_json::to_value(candidate).unwrap_or(Value::Null);
    normalize(source.get(field).cloned().unwrap_or(Value::Null))
        == normalize(candidate.get(field).cloned().unwrap_or(Value::Null))
}

fn normalize(mut value: Value) -> Value {
    match &mut value {
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| *value = normalize(value.take())),
        Value::Object(values) => {
            for key in [
                "runId",
                "eventId",
                "observationId",
                "correlationId",
                "callId",
                "requestId",
            ] {
                values.remove(key);
            }
            for value in values.values_mut() {
                *value = normalize(value.take());
            }
        }
        _ => {}
    }
    value
}
