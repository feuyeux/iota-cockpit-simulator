use cockpit_agent::{
    AgentActionBatch, AgentLifecycle, MultiAgentCoordinator, OpenWorldCheckpoint, OpenWorldRuntime,
};
use cockpit_scenario::load_scenario;
use cockpit_world::{ActionRequest, ActionStatus, DynamicEntity, HumanState, Simulation};

fn action(agent_id: &str, request_id: &str, version: u64) -> ActionRequest {
    ActionRequest {
        request_id: request_id.to_string(),
        agent_id: agent_id.to_string(),
        target: "engine-1".to_string(),
        capability_id: "engine.shutdown".to_string(),
        expected_state_version: version,
        expires_at_tick: 3,
        correlation_id: format!("{request_id}-correlation"),
    }
}

#[test]
fn multi_agent_arbitration_is_priority_then_stable_and_conflict_safe() {
    let mut scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    scenario.agents.push(cockpit_world::AgentGrant {
        agent_id: "copilot-agent".to_string(),
        capabilities: vec!["engine.shutdown".to_string()],
    });
    let mut simulation = Simulation::new("multi-agent-run", scenario);
    simulation.start().expect("run starts");
    let version = simulation.snapshot.version;
    let results = MultiAgentCoordinator.submit_batches(
        &mut simulation,
        vec![
            AgentActionBatch {
                agent_id: "copilot-agent".to_string(),
                priority: 10,
                actions: vec![action("copilot-agent", "copilot-shutdown", version)],
            },
            AgentActionBatch {
                agent_id: "cockpit-agent".to_string(),
                priority: 1,
                actions: vec![action("cockpit-agent", "pilot-shutdown", version)],
            },
        ],
    );

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].status, ActionStatus::Applied);
    assert_eq!(results[1].status, ActionStatus::Rejected);
    assert_eq!(
        results[1].error_code,
        Some(cockpit_world::ErrorCode::ActionConflict)
    );
    simulation.step_without_agent().expect("tick commits");
    assert!(simulation.snapshot.device("engine-1").unwrap().shutdown);
}

#[test]
fn actions_with_overlapping_effects_are_conflict_safe() {
    let mut scenario =
        load_scenario("scenarios/driver-fatigue-guardian.yaml").expect("scenario loads");
    scenario.agent.capabilities.extend([
        "driver.activateFatigueIntervention".to_string(),
        "privacy.activateMode".to_string(),
    ]);
    scenario.agents[0].capabilities = scenario.agent.capabilities.clone();
    let mut simulation = Simulation::new("overlapping-effects", scenario);
    simulation.start().expect("starts");
    let version = simulation.snapshot.version;
    let first = simulation.submit_action(ActionRequest {
        request_id: "fatigue".to_string(),
        agent_id: "cockpit-agent".to_string(),
        target: "dms-1".to_string(),
        capability_id: "driver.activateFatigueIntervention".to_string(),
        expected_state_version: version,
        expires_at_tick: 3,
        correlation_id: "fatigue".to_string(),
    });
    let second = simulation.submit_action(ActionRequest {
        request_id: "privacy".to_string(),
        agent_id: "cockpit-agent".to_string(),
        target: "voice-array-1".to_string(),
        capability_id: "privacy.activateMode".to_string(),
        expected_state_version: version,
        expires_at_tick: 3,
        correlation_id: "privacy".to_string(),
    });

    assert_eq!(first.status, ActionStatus::Applied);
    assert_eq!(second.status, ActionStatus::Rejected);
    assert_eq!(
        second.error_code,
        Some(cockpit_world::ErrorCode::ActionConflict)
    );
}

#[test]
fn dynamic_entities_receive_independent_sessions_and_can_be_retired() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("dynamic-world", scenario);
    simulation.start().expect("dynamic world starts");
    let mut guest = HumanState::new("guest-1");
    guest.goal = "find a safe place".to_string();
    simulation
        .spawn_entity(DynamicEntity::Human(guest))
        .expect("guest spawns");
    assert!(simulation.snapshot.human("guest-1").is_some());
    let spawn_step = simulation
        .step_without_agent()
        .expect("spawn event commits");
    assert!(spawn_step.events.iter().any(|event| {
        event.event_type == "EntitySpawned" && event.payload.target.as_deref() == Some("guest-1")
    }));

    let mut runtime = OpenWorldRuntime {
        concurrent_agent_budget: 1,
        ..OpenWorldRuntime::default()
    };
    runtime.ensure_agent("pilot-1", "protect occupants", 0);
    runtime.ensure_agent("guest-1", "find a safe place", 0);
    runtime.sessions.get_mut("guest-1").unwrap().budget.priority = 5;
    assert_ne!(
        runtime.sessions["pilot-1"].session_id,
        runtime.sessions["guest-1"].session_id
    );
    assert_eq!(
        runtime.schedule(&["pilot-1".to_string(), "guest-1".to_string()], 0),
        ["guest-1"]
    );

    runtime.record_failure("guest-1", 1, "sensor unavailable");
    for round in 0..70 {
        runtime.record_acp_turn(
            "guest-1",
            1,
            round,
            "hermes".to_string(),
            Some(format!("backend-session-guest-{round}")),
            "toolCall".to_string(),
            Some("simulation.get_observation".to_string()),
        );
    }
    runtime.replan("guest-1", 1, "wait and observe again");
    let restored = OpenWorldRuntime::restore(&runtime.sleep().expect("runtime sleeps"))
        .expect("runtime restores");
    assert_eq!(restored.sessions["guest-1"].replan_count, 1);
    assert_eq!(
        restored.sessions["guest-1"].backend_session_id.as_deref(),
        Some("backend-session-guest-69")
    );
    assert_eq!(restored.sessions["guest-1"].acp_conversation.len(), 64);
    assert!(
        restored.sessions["guest-1"].conversation_recall(20)[0]
            .contains("simulation.get_observation")
    );
    assert_eq!(
        restored.sessions["guest-1"].lifecycle,
        AgentLifecycle::Waiting
    );
    let checkpoint = OpenWorldCheckpoint::capture(&simulation.snapshot, &restored);
    let checkpoint = OpenWorldCheckpoint::decode(&checkpoint.encode().expect("checkpoint encodes"))
        .expect("checkpoint decodes");
    assert!(checkpoint.world.human("guest-1").is_some());
    assert_eq!(checkpoint.runtime.sessions["guest-1"].replan_count, 1);

    simulation.remove_entity("guest-1").expect("guest removed");
    let remove_step = simulation
        .step_without_agent()
        .expect("remove event commits");
    assert!(remove_step.events.iter().any(|event| {
        event.event_type == "EntityRemoved" && event.payload.target.as_deref() == Some("guest-1")
    }));
    runtime.retire_agent("guest-1", 2);
    assert!(simulation.snapshot.human("guest-1").is_none());
    assert_eq!(
        runtime.sessions["guest-1"].lifecycle,
        AgentLifecycle::Completed
    );
    let stable_session_id = runtime.sessions["guest-1"].session_id.clone();
    runtime.schedule(&[], 3);
    assert_eq!(
        runtime.sessions["guest-1"].lifecycle,
        AgentLifecycle::Completed,
        "absent scheduling must not overwrite a terminal retirement"
    );
    runtime.ensure_agent("guest-1", "return safely", 4);
    assert_eq!(
        runtime.sessions["guest-1"].lifecycle,
        AgentLifecycle::Active
    );
    assert_eq!(runtime.sessions["guest-1"].session_id, stable_session_id);

    runtime.ensure_agent("sleeper-1", "wait for a safe opening", 4);
    runtime.schedule(&[], 5);
    assert_eq!(
        runtime.sessions["sleeper-1"].lifecycle,
        AgentLifecycle::Sleeping
    );
    assert_eq!(
        runtime.schedule(&["sleeper-1".to_string()], 6),
        ["sleeper-1"]
    );
    assert_eq!(
        runtime.sessions["sleeper-1"].lifecycle,
        AgentLifecycle::Active
    );
}
