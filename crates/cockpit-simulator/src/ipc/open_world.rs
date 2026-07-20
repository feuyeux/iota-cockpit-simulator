//! `SimulatorHandler` methods for the open-world dynamic entity/agent-goal IPC
//! surface (`SpawnEntity`, `RemoveEntity`, `AddAgentGoal`,
//! `SetAgentGoalStatus`, `WaitAgentUntil`, `CheckpointOpenWorld`).
//!
//! Split out of `handler.rs` to separate open-world lifecycle control from
//! run lifecycle (`lifecycle.rs`) and action/plugin/recording control
//! (`control.rs`); this is a pure reorganization with no behavior changes.

use cockpit_agent::GoalStatus;
use cockpit_world::DynamicEntity;
use serde_json::json;

use super::handler::{HandlerResult, SimulatorHandler};
use super::proto::IpcError;

impl SimulatorHandler {
    pub(super) fn spawn_entity(&mut self, entity: DynamicEntity) -> HandlerResult {
        let (entity_id, human_goal) = match &entity {
            DynamicEntity::Human(human) => (human.id.clone(), Some(human.goal.clone())),
            DynamicEntity::Device(device) => (device.id.clone(), None),
        };
        let tick = {
            let simulation = self
                .simulation
                .as_mut()
                .ok_or_else(|| Box::new(Self::no_run_error()))?;
            simulation
                .spawn_entity(entity)
                .map_err(|error| Box::new(Self::simulation_error(error, Some(simulation))))?;
            simulation.snapshot.tick
        };
        if let Some(goal) = human_goal {
            self.live_driver
                .open_world_mut()
                .ensure_agent(&entity_id, &goal, tick);
        }
        self.checkpoint_open_world()?;
        Ok(json!({ "entityId": entity_id, "tick": tick, "spawned": true }))
    }

    pub(super) fn remove_entity(&mut self, entity_id: &str) -> HandlerResult {
        let (removed, tick) = {
            let simulation = self
                .simulation
                .as_mut()
                .ok_or_else(|| Box::new(Self::no_run_error()))?;
            let removed = simulation
                .remove_entity(entity_id)
                .map_err(|error| Box::new(Self::simulation_error(error, Some(simulation))))?;
            (removed, simulation.snapshot.tick)
        };
        if matches!(removed, DynamicEntity::Human(_)) {
            self.live_driver
                .open_world_mut()
                .retire_agent(entity_id, tick);
        }
        self.checkpoint_open_world()?;
        Ok(json!({ "entityId": entity_id, "tick": tick, "removed": true }))
    }

    pub(super) fn add_agent_goal(
        &mut self,
        agent_id: &str,
        description: String,
        priority: i32,
    ) -> HandlerResult {
        let description = description.trim();
        if description.is_empty() || description.len() > 512 || !(-100..=100).contains(&priority) {
            return Err(Box::new(IpcError {
                code: "OPEN_WORLD_CONTROL_INVALID".to_string(),
                message: "goal description must be 1..=512 bytes and priority -100..=100"
                    .to_string(),
                details: None,
                run_id: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.run_id().to_string()),
                tick: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.snapshot.tick),
                correlation_id: "open-world-control".to_string(),
            }));
        }
        let (tick, initial_goal, run_id) = {
            let simulation = self
                .simulation
                .as_ref()
                .ok_or_else(|| Box::new(Self::no_run_error()))?;
            (
                simulation.snapshot.tick,
                simulation
                    .snapshot
                    .human(agent_id)
                    .map(|human| human.goal.clone()),
                simulation.run_id().to_string(),
            )
        };
        if !self
            .live_driver
            .open_world()
            .sessions
            .contains_key(agent_id)
            && let Some(initial_goal) = initial_goal
        {
            self.live_driver
                .open_world_mut()
                .ensure_agent(agent_id, &initial_goal, tick);
        }
        let goal_id =
            self.live_driver
                .open_world_mut()
                .add_goal(agent_id, description, priority, tick);
        let Some(goal_id) = goal_id else {
            return Err(Box::new(IpcError {
                code: "OPEN_WORLD_AGENT_OR_CAPACITY_NOT_FOUND".to_string(),
                message: "agent was not found or its Goal capacity is exhausted".to_string(),
                details: None,
                run_id: Some(run_id),
                tick: Some(tick),
                correlation_id: "open-world-control".to_string(),
            }));
        };
        self.checkpoint_open_world()?;
        Ok(json!({ "agentId": agent_id, "goalId": goal_id, "tick": tick }))
    }

    pub(super) fn set_agent_goal_status(
        &mut self,
        agent_id: &str,
        goal_id: &str,
        status: GoalStatus,
    ) -> HandlerResult {
        let tick = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?
            .snapshot
            .tick;
        if !self
            .live_driver
            .open_world_mut()
            .set_goal_status(agent_id, goal_id, status, tick)
        {
            return Err(Box::new(IpcError {
                code: "OPEN_WORLD_GOAL_NOT_FOUND".to_string(),
                message: "agent Goal was not found".to_string(),
                details: None,
                run_id: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.run_id().to_string()),
                tick: Some(tick),
                correlation_id: "open-world-control".to_string(),
            }));
        }
        self.checkpoint_open_world()?;
        Ok(json!({ "agentId": agent_id, "goalId": goal_id, "tick": tick }))
    }

    pub(super) fn wait_agent_until(&mut self, agent_id: &str, wake_tick: u64) -> HandlerResult {
        let tick = self
            .simulation
            .as_ref()
            .ok_or_else(|| Box::new(Self::no_run_error()))?
            .snapshot
            .tick;
        if wake_tick <= tick || wake_tick > tick.saturating_add(1_000_000) {
            return Err(Box::new(IpcError {
                code: "OPEN_WORLD_WAKE_TICK_INVALID".to_string(),
                message: "wake tick must be in the bounded future".to_string(),
                details: None,
                run_id: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.run_id().to_string()),
                tick: Some(tick),
                correlation_id: "open-world-control".to_string(),
            }));
        }
        if !self
            .live_driver
            .open_world()
            .sessions
            .contains_key(agent_id)
        {
            return Err(Box::new(IpcError {
                code: "OPEN_WORLD_AGENT_NOT_FOUND".to_string(),
                message: "agent was not found".to_string(),
                details: None,
                run_id: self
                    .simulation
                    .as_ref()
                    .map(|simulation| simulation.run_id().to_string()),
                tick: Some(tick),
                correlation_id: "open-world-control".to_string(),
            }));
        }
        self.live_driver
            .open_world_mut()
            .wait_until(agent_id, wake_tick);
        self.checkpoint_open_world()?;
        Ok(json!({ "agentId": agent_id, "wakeAtTick": wake_tick, "tick": tick }))
    }

    pub(super) fn checkpoint_open_world(&mut self) -> HandlerResult {
        let checkpoint = {
            let simulation = self
                .simulation
                .as_ref()
                .ok_or_else(|| Box::new(Self::no_run_error()))?;
            self.live_driver.checkpoint(simulation)
        };
        let tick = checkpoint.world.tick;
        let agent_count = checkpoint.runtime.sessions.len();
        if let Some(recording) = self.recording.as_mut() {
            recording.open_world_checkpoint = Some(checkpoint);
        }
        self.persist_recording()?;
        Ok(json!({
            "checkpointVersion": cockpit_agent::open_world::OPEN_WORLD_RUNTIME_VERSION,
            "tick": tick,
            "agents": agent_count,
            "persisted": self.recording_store.is_some()
        }))
    }
}
