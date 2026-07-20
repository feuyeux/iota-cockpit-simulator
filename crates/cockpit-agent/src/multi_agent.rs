use cockpit_world::{ActionRequest, ActionResult, Simulation};

#[derive(Debug, Clone)]
pub struct AgentActionBatch {
    pub agent_id: String,
    pub priority: u32,
    pub actions: Vec<ActionRequest>,
}

#[derive(Debug, Default)]
pub struct MultiAgentCoordinator;

impl MultiAgentCoordinator {
    pub fn submit_batches(
        &self,
        simulation: &mut Simulation,
        mut batches: Vec<AgentActionBatch>,
    ) -> Vec<ActionResult> {
        batches.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then(left.agent_id.cmp(&right.agent_id))
        });
        let mut actions = Vec::new();
        for batch in batches {
            let mut batch_actions = batch.actions;
            batch_actions.sort_by(|left, right| {
                left.target
                    .cmp(&right.target)
                    .then(left.request_id.cmp(&right.request_id))
            });
            actions.extend(batch_actions);
        }
        actions
            .into_iter()
            .map(|action| simulation.submit_action(action))
            .collect()
    }
}
