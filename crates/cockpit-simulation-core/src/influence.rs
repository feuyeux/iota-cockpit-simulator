//! Scheduled influence rules, deterministic conflict arbitration, and event
//! subscriptions beyond the smoke scenario's direct systems.
//!
//! An [`InfluenceRule`] declares a versioned, scheduled mutation of one world
//! component. On each tick the [`schedule_due`] selector picks the rules due at
//! that tick, and [`arbitrate`] resolves conflicts — multiple rules targeting
//! the same `(entity, component)` in the same tick — deterministically under an
//! explicit [`ConflictPolicy`]. [`Subscription::deliver`] filters events for a
//! subscriber by event type in stable order.
//!
//! All ordering is total and content-derived so replays are bit-stable. When a
//! scenario declares no influences, none of this changes tick behavior.

use serde::{Deserialize, Serialize};

/// When an influence rule fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum InfluenceSchedule {
    /// Fires once at exactly this tick.
    AtTick { tick: u64 },
    /// Fires at `start` and then every `interval` ticks (interval >= 1).
    Every { start: u64, interval: u64 },
}

impl InfluenceSchedule {
    /// Whether the rule is due at `tick`.
    pub fn is_due(&self, tick: u64) -> bool {
        match *self {
            InfluenceSchedule::AtTick { tick: at } => tick == at,
            InfluenceSchedule::Every { start, interval } => {
                let interval = interval.max(1);
                tick >= start && (tick - start).is_multiple_of(interval)
            }
        }
    }
}

/// How an influence rule changes its target component's numeric value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op", content = "value")]
pub enum InfluenceOp {
    /// Replace the component value.
    Set(f64),
    /// Add a signed delta to the current component value.
    Delta(f64),
}

impl InfluenceOp {
    /// Resolve the target value given the current value.
    pub fn resolve(&self, current: f64) -> f64 {
        match *self {
            InfluenceOp::Set(value) => value,
            InfluenceOp::Delta(delta) => current + delta,
        }
    }
}

/// A versioned, scheduled influence over a single world component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceRule {
    pub rule_id: String,
    /// Rule schema version, recorded so replays can reject incompatible rules.
    pub rule_version: u32,
    pub schedule: InfluenceSchedule,
    pub entity_id: String,
    pub component_path: String,
    pub op: InfluenceOp,
    /// Higher priority wins under `HighestPriorityWins`.
    #[serde(default)]
    pub priority: i32,
}

/// How to resolve multiple rules targeting the same component in one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    /// Reject all conflicting rules for the contested component this tick.
    RejectConflicting,
    /// Apply the highest-priority rule; ties broken by lowest `rule_id`.
    HighestPriorityWins,
    /// Apply the rule with the lexicographically lowest `rule_id`.
    LowestRuleIdWins,
}

/// The disposition of one influence rule after arbitration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceDecision {
    pub rule_id: String,
    pub entity_id: String,
    pub component_path: String,
    pub applied: bool,
    /// Present when `applied` is false: why the rule lost arbitration.
    pub rejected_reason: Option<String>,
}

/// Result of arbitrating the due rules for a tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArbitrationOutcome {
    /// Winning rules in deterministic `(entity, component)` order.
    pub winners: Vec<InfluenceRule>,
    /// Per-rule decisions (winners and losers), for recording evidence.
    pub decisions: Vec<InfluenceDecision>,
}

/// Select the rules due at `tick`, in deterministic order.
pub fn schedule_due(rules: &[InfluenceRule], tick: u64) -> Vec<InfluenceRule> {
    let mut due: Vec<InfluenceRule> = rules
        .iter()
        .filter(|rule| rule.schedule.is_due(tick))
        .cloned()
        .collect();
    due.sort_by(compare_rules);
    due
}

/// Total, content-derived ordering over rules: by target, then priority
/// (descending), then rule id.
fn compare_rules(left: &InfluenceRule, right: &InfluenceRule) -> std::cmp::Ordering {
    left.entity_id
        .cmp(&right.entity_id)
        .then(left.component_path.cmp(&right.component_path))
        .then(right.priority.cmp(&left.priority))
        .then(left.rule_id.cmp(&right.rule_id))
}

/// Arbitrate the due rules under `policy`, returning winners and per-rule
/// decisions. Input need not be pre-sorted; output ordering is deterministic.
pub fn arbitrate(due: &[InfluenceRule], policy: ConflictPolicy) -> ArbitrationOutcome {
    // Group by target component while preserving deterministic order.
    let mut sorted = due.to_vec();
    sorted.sort_by(compare_rules);

    let mut outcome = ArbitrationOutcome::default();
    let mut index = 0;
    while index < sorted.len() {
        let target = (
            sorted[index].entity_id.clone(),
            sorted[index].component_path.clone(),
        );
        let mut group_end = index + 1;
        while group_end < sorted.len()
            && sorted[group_end].entity_id == target.0
            && sorted[group_end].component_path == target.1
        {
            group_end += 1;
        }
        let group = &sorted[index..group_end];
        resolve_group(group, policy, &mut outcome);
        index = group_end;
    }
    outcome
}

fn resolve_group(
    group: &[InfluenceRule],
    policy: ConflictPolicy,
    outcome: &mut ArbitrationOutcome,
) {
    if group.len() == 1 {
        outcome.winners.push(group[0].clone());
        outcome.decisions.push(decision(&group[0], true, None));
        return;
    }
    match policy {
        ConflictPolicy::RejectConflicting => {
            for rule in group {
                outcome.decisions.push(decision(
                    rule,
                    false,
                    Some("conflicting rules rejected for contested component".to_string()),
                ));
            }
        }
        ConflictPolicy::HighestPriorityWins | ConflictPolicy::LowestRuleIdWins => {
            // `group` is already sorted by priority-desc then rule_id, so for
            // HighestPriorityWins the winner is index 0. For LowestRuleIdWins
            // pick the lexicographically smallest rule_id.
            let winner_index = match policy {
                ConflictPolicy::HighestPriorityWins => 0,
                ConflictPolicy::LowestRuleIdWins => group
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.rule_id.cmp(&b.rule_id))
                    .map(|(i, _)| i)
                    .unwrap_or(0),
                ConflictPolicy::RejectConflicting => unreachable!(),
            };
            for (i, rule) in group.iter().enumerate() {
                if i == winner_index {
                    outcome.winners.push(rule.clone());
                    outcome.decisions.push(decision(rule, true, None));
                } else {
                    outcome.decisions.push(decision(
                        rule,
                        false,
                        Some("lost deterministic arbitration".to_string()),
                    ));
                }
            }
        }
    }
}

fn decision(rule: &InfluenceRule, applied: bool, reason: Option<String>) -> InfluenceDecision {
    InfluenceDecision {
        rule_id: rule.rule_id.clone(),
        entity_id: rule.entity_id.clone(),
        component_path: rule.component_path.clone(),
        applied,
        rejected_reason: reason,
    }
}

/// A subscription that selects events by type for a named subscriber.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub subscriber_id: String,
    /// Event types this subscriber receives. Empty means all types.
    #[serde(default)]
    pub event_types: Vec<String>,
}

impl Subscription {
    /// Whether an event of `event_type` matches this subscription.
    pub fn matches(&self, event_type: &str) -> bool {
        self.event_types.is_empty() || self.event_types.iter().any(|value| value == event_type)
    }

    /// Deliver the matching event indices from `event_types_in_order`,
    /// preserving input order (which is already deterministic per tick).
    pub fn deliver<'a, I>(&self, events: I) -> Vec<usize>
    where
        I: IntoIterator<Item = &'a str>,
    {
        events
            .into_iter()
            .enumerate()
            .filter(|(_, event_type)| self.matches(event_type))
            .map(|(index, _)| index)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, priority: i32, op: InfluenceOp) -> InfluenceRule {
        InfluenceRule {
            rule_id: id.to_string(),
            rule_version: 1,
            schedule: InfluenceSchedule::AtTick { tick: 5 },
            entity_id: "cabin".to_string(),
            component_path: "environment.smokeDensity".to_string(),
            op,
            priority,
        }
    }

    #[test]
    fn at_tick_and_every_schedules_fire_correctly() {
        let at = InfluenceSchedule::AtTick { tick: 3 };
        assert!(at.is_due(3));
        assert!(!at.is_due(4));

        let every = InfluenceSchedule::Every { start: 2, interval: 3 };
        assert!(every.is_due(2));
        assert!(every.is_due(5));
        assert!(every.is_due(8));
        assert!(!every.is_due(3));
        assert!(!every.is_due(1));
    }

    #[test]
    fn every_with_zero_interval_is_treated_as_one() {
        let every = InfluenceSchedule::Every { start: 0, interval: 0 };
        assert!(every.is_due(0));
        assert!(every.is_due(1));
    }

    #[test]
    fn ops_resolve_against_current_value() {
        assert_eq!(InfluenceOp::Set(0.5).resolve(0.9), 0.5);
        assert_eq!(InfluenceOp::Delta(0.2).resolve(0.3), 0.5);
    }

    #[test]
    fn schedule_due_selects_and_orders_deterministically() {
        let rules = vec![
            rule("b", 0, InfluenceOp::Set(0.5)),
            rule("a", 0, InfluenceOp::Set(0.6)),
        ];
        let due = schedule_due(&rules, 5);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].rule_id, "a");
        assert_eq!(due[1].rule_id, "b");
    }

    #[test]
    fn no_conflict_applies_single_rule() {
        let due = vec![rule("a", 0, InfluenceOp::Set(0.5))];
        let outcome = arbitrate(&due, ConflictPolicy::RejectConflicting);
        assert_eq!(outcome.winners.len(), 1);
        assert!(outcome.decisions[0].applied);
    }

    #[test]
    fn reject_conflicting_drops_all_contenders() {
        let due = vec![
            rule("a", 0, InfluenceOp::Set(0.5)),
            rule("b", 5, InfluenceOp::Set(0.9)),
        ];
        let outcome = arbitrate(&due, ConflictPolicy::RejectConflicting);
        assert!(outcome.winners.is_empty());
        assert_eq!(outcome.decisions.len(), 2);
        assert!(outcome.decisions.iter().all(|decision| !decision.applied));
    }

    #[test]
    fn highest_priority_wins_with_rule_id_tiebreak() {
        let due = vec![
            rule("a", 1, InfluenceOp::Set(0.5)),
            rule("b", 9, InfluenceOp::Set(0.9)),
            rule("c", 9, InfluenceOp::Set(0.7)),
        ];
        let outcome = arbitrate(&due, ConflictPolicy::HighestPriorityWins);
        assert_eq!(outcome.winners.len(), 1);
        // Priority 9 ties between b and c; lowest rule_id "b" wins.
        assert_eq!(outcome.winners[0].rule_id, "b");
        let applied: Vec<_> = outcome
            .decisions
            .iter()
            .filter(|decision| decision.applied)
            .map(|decision| decision.rule_id.clone())
            .collect();
        assert_eq!(applied, vec!["b"]);
    }

    #[test]
    fn lowest_rule_id_wins_ignores_priority() {
        let due = vec![
            rule("z", 9, InfluenceOp::Set(0.5)),
            rule("a", 0, InfluenceOp::Set(0.9)),
        ];
        let outcome = arbitrate(&due, ConflictPolicy::LowestRuleIdWins);
        assert_eq!(outcome.winners.len(), 1);
        assert_eq!(outcome.winners[0].rule_id, "a");
    }

    #[test]
    fn distinct_components_do_not_conflict() {
        let mut other = rule("b", 0, InfluenceOp::Set(0.5));
        other.component_path = "environment.visibility".to_string();
        let due = vec![rule("a", 0, InfluenceOp::Set(0.5)), other];
        let outcome = arbitrate(&due, ConflictPolicy::RejectConflicting);
        assert_eq!(outcome.winners.len(), 2);
    }

    #[test]
    fn subscription_filters_by_event_type() {
        let all = Subscription {
            subscriber_id: "all".to_string(),
            event_types: vec![],
        };
        let smoke = Subscription {
            subscriber_id: "smoke".to_string(),
            event_types: vec!["SmokeDetected".to_string()],
        };
        let events = ["SmokeDetected", "StressChanged", "SmokeDetected"];
        assert_eq!(all.deliver(events), vec![0, 1, 2]);
        assert_eq!(smoke.deliver(events), vec![0, 2]);
        assert!(smoke.matches("SmokeDetected"));
        assert!(!smoke.matches("StressChanged"));
    }
}
