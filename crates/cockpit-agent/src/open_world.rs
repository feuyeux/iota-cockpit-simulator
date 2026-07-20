use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{HumanDecision, HumanToolCall};

pub const OPEN_WORLD_RUNTIME_VERSION: u32 = 3;
pub const DEFAULT_CONCURRENT_AGENT_BUDGET: usize = 8;
pub const DEFAULT_AGENT_TOOL_BUDGET: u32 = 16;
pub const MAX_EPISODIC_MEMORIES_PER_AGENT: usize = 256;
pub const MAX_GOALS_PER_AGENT: usize = 64;
pub const MAX_ACP_CONVERSATION_TURNS_PER_AGENT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConversationTurn {
    pub tick: u64,
    pub round: usize,
    pub backend: String,
    pub backend_session_id: Option<String>,
    pub response_kind: String,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentLifecycle {
    Active,
    Waiting,
    Sleeping,
    Recovering,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalStatus {
    Proposed,
    Active,
    Satisfied,
    Blocked,
    Abandoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanStepStatus {
    Pending,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceLifecycle {
    Registered,
    Active,
    Suspended,
    Failed,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillState {
    pub skill_id: String,
    pub version: String,
    pub lifecycle: ResourceLifecycle,
    pub activated_tick: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolState {
    pub tool_name: String,
    pub lifecycle: ResourceLifecycle,
    pub weighted_cost: u32,
    pub call_count: u64,
    pub failure_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalState {
    pub goal_id: String,
    pub description: String,
    pub priority: i32,
    pub status: GoalStatus,
    pub created_tick: u64,
    pub updated_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub step_id: String,
    pub description: String,
    pub required_tool: Option<String>,
    pub status: PlanStepStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub retry_after_tick: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicMemory {
    pub tick: u64,
    pub kind: String,
    pub summary: String,
    pub related_entities: Vec<String>,
    pub importance: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipState {
    pub other_entity_id: String,
    pub trust: f64,
    pub affinity: f64,
    pub interaction_count: u64,
    pub last_interaction_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBudget {
    pub max_tool_cost_per_tick: u32,
    pub remaining_tool_cost: u32,
    pub priority: i32,
}

impl Default for AgentBudget {
    fn default() -> Self {
        Self {
            max_tool_cost_per_tick: DEFAULT_AGENT_TOOL_BUDGET,
            remaining_tool_cost: DEFAULT_AGENT_TOOL_BUDGET,
            priority: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionState {
    pub agent_id: String,
    pub session_id: String,
    #[serde(default)]
    pub backend_session_id: Option<String>,
    #[serde(default)]
    pub acp_conversation: Vec<AcpConversationTurn>,
    pub lifecycle: AgentLifecycle,
    pub goals: Vec<GoalState>,
    pub plan: Vec<PlanStep>,
    pub skills: BTreeMap<String, SkillState>,
    pub tools: BTreeMap<String, ToolState>,
    pub episodic_memory: Vec<EpisodicMemory>,
    pub relationships: BTreeMap<String, RelationshipState>,
    pub budget: AgentBudget,
    pub wake_at_tick: Option<u64>,
    pub consecutive_failures: u32,
    pub replan_count: u32,
    pub last_active_tick: u64,
}

impl AgentSessionState {
    fn new(agent_id: &str, initial_goal: &str, tick: u64) -> Self {
        let tools = [
            crate::TOOL_GET_TURN_CONTEXT,
            crate::TOOL_GET_OBSERVATION,
            crate::TOOL_LIST_VISIBLE_ENTITIES,
            crate::TOOL_INSPECT_SENSOR_QUALITY,
            crate::TOOL_REQUEST_ACTION,
            crate::TOOL_GET_ACTION_RESULT,
            crate::TOOL_GET_RUN_STATUS,
            crate::TOOL_ADD_GOAL,
            crate::TOOL_WAIT_UNTIL,
        ]
        .into_iter()
        .map(|tool_name| {
            (
                tool_name.to_string(),
                ToolState {
                    tool_name: tool_name.to_string(),
                    lifecycle: ResourceLifecycle::Active,
                    weighted_cost: match tool_name {
                        crate::TOOL_REQUEST_ACTION => 4,
                        crate::TOOL_ADD_GOAL | crate::TOOL_WAIT_UNTIL => 2,
                        _ => 1,
                    },
                    call_count: 0,
                    failure_count: 0,
                },
            )
        })
        .collect();
        let skills = [(
            "cockpit-world".to_string(),
            SkillState {
                skill_id: "cockpit-world".to_string(),
                version: crate::skill::COCKPIT_SKILL_VERSION.to_string(),
                lifecycle: ResourceLifecycle::Active,
                activated_tick: Some(tick),
                last_error: None,
            },
        )]
        .into_iter()
        .collect();
        Self {
            agent_id: agent_id.to_string(),
            session_id: format!("agent-session-{agent_id}"),
            backend_session_id: None,
            acp_conversation: Vec::new(),
            lifecycle: AgentLifecycle::Active,
            goals: vec![GoalState {
                goal_id: format!("goal-{agent_id}-1"),
                description: initial_goal.to_string(),
                priority: 0,
                status: GoalStatus::Active,
                created_tick: tick,
                updated_tick: tick,
            }],
            plan: vec![PlanStep {
                step_id: format!("plan-{agent_id}-observe"),
                description: "Observe the world and choose the next safe action".to_string(),
                required_tool: Some(crate::TOOL_GET_TURN_CONTEXT.to_string()),
                status: PlanStepStatus::Pending,
                attempts: 0,
                max_attempts: 3,
                retry_after_tick: None,
                last_error: None,
            }],
            skills,
            tools,
            episodic_memory: Vec::new(),
            relationships: BTreeMap::new(),
            budget: AgentBudget::default(),
            wake_at_tick: None,
            consecutive_failures: 0,
            replan_count: 0,
            last_active_tick: tick,
        }
    }

    pub fn recall(&self, limit: usize) -> Vec<String> {
        self.episodic_memory
            .iter()
            .rev()
            .take(limit)
            .map(|memory| format!("tick {} [{}] {}", memory.tick, memory.kind, memory.summary))
            .collect()
    }

    pub fn conversation_recall(&self, limit: usize) -> Vec<String> {
        self.acp_conversation
            .iter()
            .rev()
            .take(limit)
            .rev()
            .map(|turn| {
                format!(
                    "ACP tick {} round {} backend={} response={}{}",
                    turn.tick,
                    turn.round,
                    turn.backend,
                    turn.response_kind,
                    turn.tool_name
                        .as_ref()
                        .map(|tool| format!(" tool={tool}"))
                        .unwrap_or_default()
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWorldRuntime {
    pub version: u32,
    pub sessions: BTreeMap<String, AgentSessionState>,
    pub retired_entities: BTreeSet<String>,
    pub concurrent_agent_budget: usize,
    pub scheduler_epoch: u64,
}

impl Default for OpenWorldRuntime {
    fn default() -> Self {
        Self {
            version: OPEN_WORLD_RUNTIME_VERSION,
            sessions: BTreeMap::new(),
            retired_entities: BTreeSet::new(),
            concurrent_agent_budget: DEFAULT_CONCURRENT_AGENT_BUDGET,
            scheduler_epoch: 0,
        }
    }
}

impl OpenWorldRuntime {
    pub fn ensure_agent(&mut self, agent_id: &str, initial_goal: &str, tick: u64) {
        let was_retired = self.retired_entities.remove(agent_id);
        let session = self
            .sessions
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentSessionState::new(agent_id, initial_goal, tick));
        if was_retired {
            session.lifecycle = AgentLifecycle::Active;
            session.last_active_tick = tick;
        }
    }

    pub fn retire_agent(&mut self, agent_id: &str, tick: u64) {
        if let Some(session) = self.sessions.get_mut(agent_id) {
            session.lifecycle = AgentLifecycle::Completed;
            session.last_active_tick = tick;
        }
        self.retired_entities.insert(agent_id.to_string());
    }

    pub fn add_goal(
        &mut self,
        agent_id: &str,
        description: impl Into<String>,
        priority: i32,
        tick: u64,
    ) -> Option<String> {
        let session = self.sessions.get_mut(agent_id)?;
        if session.goals.len() >= MAX_GOALS_PER_AGENT {
            return None;
        }
        let goal_id = format!("goal-{agent_id}-{}", session.goals.len() + 1);
        session.goals.push(GoalState {
            goal_id: goal_id.clone(),
            description: description.into(),
            priority,
            status: GoalStatus::Proposed,
            created_tick: tick,
            updated_tick: tick,
        });
        Some(goal_id)
    }

    pub fn set_goal_status(
        &mut self,
        agent_id: &str,
        goal_id: &str,
        status: GoalStatus,
        tick: u64,
    ) -> bool {
        let Some(goal) = self.sessions.get_mut(agent_id).and_then(|session| {
            session
                .goals
                .iter_mut()
                .find(|goal| goal.goal_id == goal_id)
        }) else {
            return false;
        };
        goal.status = status;
        goal.updated_tick = tick;
        true
    }

    pub fn schedule(&mut self, active_agents: &[String], tick: u64) -> Vec<String> {
        self.scheduler_epoch = self.scheduler_epoch.saturating_add(1);
        let active = active_agents
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for (id, session) in &mut self.sessions {
            if active.contains(id.as_str()) && session.lifecycle == AgentLifecycle::Sleeping {
                session.lifecycle = AgentLifecycle::Active;
            } else if !active.contains(id.as_str())
                && !matches!(
                    session.lifecycle,
                    AgentLifecycle::Completed | AgentLifecycle::Failed
                )
            {
                session.lifecycle = AgentLifecycle::Sleeping;
            }
            if session.lifecycle == AgentLifecycle::Waiting
                && session.wake_at_tick.is_some_and(|wake| wake <= tick)
            {
                session.lifecycle = AgentLifecycle::Recovering;
                session.wake_at_tick = None;
            }
            session.budget.remaining_tool_cost = session.budget.max_tool_cost_per_tick;
        }
        let mut scheduled = active_agents
            .iter()
            .filter_map(|id| self.sessions.get(id).map(|session| (id, session)))
            .filter(|(_, session)| {
                matches!(
                    session.lifecycle,
                    AgentLifecycle::Active | AgentLifecycle::Recovering
                )
            })
            .map(|(id, session)| {
                (
                    id.clone(),
                    session.budget.priority,
                    session.last_active_tick,
                )
            })
            .collect::<Vec<_>>();
        scheduled.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then(left.2.cmp(&right.2))
                .then(left.0.cmp(&right.0))
        });
        scheduled
            .into_iter()
            .take(self.concurrent_agent_budget.max(1))
            .map(|(id, _, _)| id)
            .collect()
    }

    pub fn wait_until(&mut self, agent_id: &str, wake_tick: u64) {
        if let Some(session) = self.sessions.get_mut(agent_id) {
            session.lifecycle = AgentLifecycle::Waiting;
            session.wake_at_tick = Some(wake_tick);
            if let Some(step) = session
                .plan
                .iter_mut()
                .find(|step| step.status == PlanStepStatus::Running)
            {
                step.status = PlanStepStatus::Waiting;
                step.retry_after_tick = Some(wake_tick);
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The persisted ACP trace has a fixed schema and is captured at one boundary."
    )]
    pub fn record_acp_turn(
        &mut self,
        agent_id: &str,
        tick: u64,
        round: usize,
        backend: String,
        backend_session_id: Option<String>,
        response_kind: String,
        tool_name: Option<String>,
    ) {
        let Some(session) = self.sessions.get_mut(agent_id) else {
            return;
        };
        if backend_session_id.is_some() {
            session.backend_session_id = backend_session_id.clone();
        }
        session.acp_conversation.push(AcpConversationTurn {
            tick,
            round,
            backend,
            backend_session_id,
            response_kind,
            tool_name,
        });
        if session.acp_conversation.len() > MAX_ACP_CONVERSATION_TURNS_PER_AGENT {
            let remove = session.acp_conversation.len() - MAX_ACP_CONVERSATION_TURNS_PER_AGENT;
            session.acp_conversation.drain(..remove);
        }
    }

    pub fn record_failure(&mut self, agent_id: &str, tick: u64, error: &str) {
        let Some(session) = self.sessions.get_mut(agent_id) else {
            return;
        };
        let failure_class = Self::failure_class(error);
        session.consecutive_failures = session.consecutive_failures.saturating_add(1);
        session.lifecycle = AgentLifecycle::Recovering;
        if let Some(step) = session.plan.iter_mut().find(|step| {
            matches!(
                step.status,
                PlanStepStatus::Running | PlanStepStatus::Pending
            )
        }) {
            step.status = PlanStepStatus::Failed;
            step.attempts = step.attempts.saturating_add(1);
            step.last_error = Some(failure_class.to_string());
        }
        session.episodic_memory.push(EpisodicMemory {
            tick,
            kind: "failure".to_string(),
            summary: format!("turn transaction failed: {failure_class}"),
            related_entities: vec![agent_id.to_string()],
            importance: 8,
        });
        Self::compact_memory(session);
    }

    pub fn replan(&mut self, agent_id: &str, tick: u64, reason: &str) {
        let Some(session) = self.sessions.get_mut(agent_id) else {
            return;
        };
        session.replan_count = session.replan_count.saturating_add(1);
        session.plan.push(PlanStep {
            step_id: format!("replan-{agent_id}-{}", session.replan_count),
            description: format!("Re-observe and recover: {reason}"),
            required_tool: Some(crate::TOOL_GET_TURN_CONTEXT.to_string()),
            status: PlanStepStatus::Pending,
            attempts: 0,
            max_attempts: 3,
            retry_after_tick: Some(tick.saturating_add(1)),
            last_error: None,
        });
        session.lifecycle = AgentLifecycle::Waiting;
        session.wake_at_tick = Some(tick.saturating_add(1));
    }

    pub fn record_turn(
        &mut self,
        agent_id: &str,
        tick: u64,
        decision: &HumanDecision,
        tools: &[HumanToolCall],
        other_agents: &[String],
    ) {
        let Some(session) = self.sessions.get_mut(agent_id) else {
            return;
        };
        session.lifecycle = AgentLifecycle::Active;
        session.last_active_tick = tick;
        session.consecutive_failures = 0;
        let spent = tools
            .iter()
            .map(|tool| match tool.tool.as_str() {
                crate::TOOL_REQUEST_ACTION => 4,
                crate::TOOL_ADD_GOAL | crate::TOOL_WAIT_UNTIL => 2,
                _ => 1,
            })
            .sum::<u32>();
        session.budget.remaining_tool_cost =
            session.budget.remaining_tool_cost.saturating_sub(spent);
        for call in tools {
            if let Some(tool) = session.tools.get_mut(&call.tool) {
                tool.call_count = tool.call_count.saturating_add(1);
            }
        }
        let summary = format!(
            "completed turn with {} tool calls; requested_action={}",
            tools.len(),
            tools
                .iter()
                .any(|tool| tool.tool == crate::TOOL_REQUEST_ACTION)
        );
        session.episodic_memory.push(EpisodicMemory {
            tick,
            kind: "turn".to_string(),
            summary,
            related_entities: other_agents.to_vec(),
            importance: if tools
                .iter()
                .any(|tool| tool.tool == crate::TOOL_REQUEST_ACTION)
            {
                8
            } else {
                4
            },
        });
        if let Some(step) = session.plan.iter_mut().find(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::Running | PlanStepStatus::Waiting
            )
        }) {
            step.status = PlanStepStatus::Succeeded;
            step.attempts = step.attempts.saturating_add(1);
        }
        if decision.utterance.is_some() {
            for other in other_agents {
                let relationship =
                    session
                        .relationships
                        .entry(other.clone())
                        .or_insert(RelationshipState {
                            other_entity_id: other.clone(),
                            trust: 0.5,
                            affinity: 0.5,
                            interaction_count: 0,
                            last_interaction_tick: tick,
                        });
                relationship.interaction_count = relationship.interaction_count.saturating_add(1);
                relationship.last_interaction_tick = tick;
                relationship.trust = (relationship.trust + 0.01).clamp(0.0, 1.0);
            }
        }
        Self::compact_memory(session);
    }

    pub fn set_skill_lifecycle(
        &mut self,
        agent_id: &str,
        skill_id: &str,
        lifecycle: ResourceLifecycle,
        tick: u64,
        error: Option<String>,
    ) {
        if let Some(skill) = self
            .sessions
            .get_mut(agent_id)
            .and_then(|session| session.skills.get_mut(skill_id))
        {
            skill.lifecycle = lifecycle;
            skill.activated_tick = (lifecycle == ResourceLifecycle::Active).then_some(tick);
            skill.last_error = error;
        }
    }

    pub fn record_tool_failure(&mut self, agent_id: &str, tool_name: &str) {
        if let Some(tool) = self
            .sessions
            .get_mut(agent_id)
            .and_then(|session| session.tools.get_mut(tool_name))
        {
            tool.failure_count = tool.failure_count.saturating_add(1);
            tool.lifecycle = ResourceLifecycle::Failed;
        }
    }

    fn failure_class(error: &str) -> &'static str {
        let normalized = error.to_ascii_lowercase();
        if normalized.contains("cancel") {
            "cancelled"
        } else if normalized.contains("timeout") || normalized.contains("wall-clock") {
            "timeout"
        } else if normalized.contains("budget") {
            "budget_exhausted"
        } else if normalized.contains("diverg") {
            "tool_result_divergence"
        } else if normalized.contains("invalid") || normalized.contains("malformed") {
            "invalid_output"
        } else {
            "backend_failure"
        }
    }

    fn compact_memory(session: &mut AgentSessionState) {
        if session.episodic_memory.len() > MAX_EPISODIC_MEMORIES_PER_AGENT {
            let remove = session.episodic_memory.len() - MAX_EPISODIC_MEMORIES_PER_AGENT;
            session.episodic_memory.drain(..remove);
        }
    }

    pub fn sleep(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn restore(bytes: &[u8]) -> Result<Self, String> {
        let runtime: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if runtime.version != OPEN_WORLD_RUNTIME_VERSION {
            return Err(format!(
                "open-world runtime version {} is incompatible with {}",
                runtime.version, OPEN_WORLD_RUNTIME_VERSION
            ));
        }
        Ok(runtime)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWorldCheckpoint {
    pub checkpoint_version: u32,
    pub world: cockpit_world::WorldSnapshot,
    pub runtime: OpenWorldRuntime,
}

impl OpenWorldCheckpoint {
    pub fn capture(world: &cockpit_world::WorldSnapshot, runtime: &OpenWorldRuntime) -> Self {
        Self {
            checkpoint_version: OPEN_WORLD_RUNTIME_VERSION,
            world: world.clone(),
            runtime: runtime.clone(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let checkpoint: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if checkpoint.checkpoint_version != OPEN_WORLD_RUNTIME_VERSION
            || checkpoint.runtime.version != OPEN_WORLD_RUNTIME_VERSION
        {
            return Err("open-world checkpoint version is incompatible".to_string());
        }
        Ok(checkpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_is_priority_bounded_and_runtime_restores() {
        let mut runtime = OpenWorldRuntime {
            concurrent_agent_budget: 1,
            ..OpenWorldRuntime::default()
        };
        runtime.ensure_agent("a", "goal a", 0);
        runtime.ensure_agent("b", "goal b", 0);
        runtime.sessions.get_mut("b").unwrap().budget.priority = 10;
        assert_eq!(
            runtime.sessions["b"].skills["cockpit-world"].lifecycle,
            ResourceLifecycle::Active
        );
        assert_eq!(runtime.sessions["b"].tools.len(), 9);
        let goal = runtime.add_goal("b", "recover safely", 5, 1).unwrap();
        assert!(runtime.set_goal_status("b", &goal, GoalStatus::Active, 1));
        runtime.record_tool_failure("b", crate::TOOL_GET_OBSERVATION);
        assert_eq!(
            runtime.sessions["b"].tools[crate::TOOL_GET_OBSERVATION].lifecycle,
            ResourceLifecycle::Failed
        );
        assert_eq!(
            runtime.schedule(&["a".to_string(), "b".to_string()], 0),
            ["b"]
        );

        runtime.record_failure("b", 1, "tool unavailable");
        runtime.replan("b", 1, "retry after observation");
        let restored = OpenWorldRuntime::restore(&runtime.sleep().unwrap()).unwrap();
        assert_eq!(restored.sessions["b"].replan_count, 1);
        assert_eq!(restored.sessions["b"].lifecycle, AgentLifecycle::Waiting);
    }
}
