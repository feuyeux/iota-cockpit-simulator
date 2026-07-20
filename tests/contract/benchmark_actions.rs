use cockpit_agent::{LocalMcpServer, RuleAgent};
use cockpit_scenario::load_scenario;
use cockpit_world::{
    ActionRequest, ActionStatus, ErrorCode, Observation, Simulation, clock::RunStatus,
};

const BENCHMARK_ACTIONS: &[(&str, &str, &str)] = &[
    (
        "scenarios/heatwave-thermal-comfort.yaml",
        "climate.restoreComfort",
        "ThermalComfortRisk",
    ),
    (
        "scenarios/winter-defog-visibility.yaml",
        "visibility.activateDefog",
        "WindshieldVisibilityRisk",
    ),
    (
        "scenarios/driver-fatigue-guardian.yaml",
        "driver.activateFatigueIntervention",
        "DriverFatigueRisk",
    ),
    (
        "scenarios/child-left-behind.yaml",
        "occupant.activateChildProtection",
        "ChildPresenceHeatRisk",
    ),
    (
        "scenarios/medical-emergency.yaml",
        "health.activateMedicalResponse",
        "MedicalEmergencyRisk",
    ),
    (
        "scenarios/voice-privacy-conflict.yaml",
        "privacy.activateMode",
        "MultiUserPrivacyConflict",
    ),
    (
        "scenarios/ev-range-anxiety.yaml",
        "energy.acceptChargingPlan",
        "EvRangeRisk",
    ),
    (
        "scenarios/adas-takeover-construction.yaml",
        "adas.acknowledgeTakeover",
        "AdasTakeoverRequired",
    ),
    (
        "scenarios/cybersecurity-anomalous-control.yaml",
        "cybersecurity.enterSafeMode",
        "CyberControlAnomaly",
    ),
];

#[test]
fn every_benchmark_domain_closes_through_a_typed_gateway_action() {
    for (path, expected_capability_id, resolved_alert) in BENCHMARK_ACTIONS {
        let scenario = load_scenario(path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let deadline = scenario.max_ticks;
        let mut simulation = Simulation::new(format!("action-{}", scenario.id), scenario);
        simulation.start().expect("simulation starts");
        let mut agent = RuleAgent::default();
        let mut server = LocalMcpServer::default();
        let mut applied = false;
        let expected_target = simulation
            .capabilities()
            .get(expected_capability_id)
            .expect("capability is registered")
            .target_id
            .clone();

        for _ in 0..=deadline {
            let step = agent
                .step(&mut simulation, &mut server)
                .unwrap_or_else(|error| panic!("{path}: {error}"));
            applied |= step.action_results.iter().any(|result| {
                result.status == ActionStatus::Applied
                    && result.request.capability_id == *expected_capability_id
                    && result.request.target == expected_target
            });
        }

        assert!(applied, "{path}: typed action was not applied");
        assert_domain_state(path, &simulation);
        let observation =
            Observation::from_snapshot(simulation.run_id(), "cockpit-agent", &simulation.snapshot);
        assert!(
            !observation
                .alerts
                .iter()
                .any(|alert| alert == resolved_alert),
            "{path}: resolved alert remained active"
        );
    }
}

fn assert_domain_state(path: &str, simulation: &Simulation) {
    let systems = &simulation.snapshot.cockpit_systems;
    let active = match path {
        "scenarios/heatwave-thermal-comfort.yaml" => {
            systems.climate.cooling_active && systems.climate.seat_ventilation_active
        }
        "scenarios/winter-defog-visibility.yaml" => systems.climate.defog_active,
        "scenarios/driver-fatigue-guardian.yaml" => {
            systems.driver_assistance.fatigue_intervention_active
        }
        "scenarios/child-left-behind.yaml" => {
            systems.occupant_care.child_protection_active
                && systems.occupant_care.guardian_notified
                && systems.occupant_care.remote_unlock_requested
        }
        "scenarios/medical-emergency.yaml" => {
            systems.occupant_care.medical_response_active
                && systems.connectivity.emergency_call_active
                && systems.mobility.emergency_route_active
        }
        "scenarios/voice-privacy-conflict.yaml" => {
            systems.experience.privacy_mode_active
                && systems.experience.media_sessions_isolated
                && systems.experience.occupant_profiles_isolated
        }
        "scenarios/ev-range-anxiety.yaml" => {
            systems.experience.charging_plan_accepted
                && systems.mobility.charging_route_active
                && systems.mobility.charger_service_connected
        }
        "scenarios/adas-takeover-construction.yaml" => {
            systems.driver_assistance.takeover_acknowledged
                && systems.driver_assistance.takeover_hmi_active
        }
        "scenarios/cybersecurity-anomalous-control.yaml" => {
            systems.cybersecurity.safe_mode_active
                && systems.cybersecurity.network_isolated
                && systems.cybersecurity.identity_verified
                && systems.connectivity.remote_services_isolated
                && systems.connectivity.trusted_local_alert_active
        }
        _ => false,
    };
    assert!(active, "{path}: authoritative system state was not updated");
}

#[test]
fn domain_action_without_a_scenario_grant_is_rejected() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("unauthorized-domain-action", scenario);
    simulation.start().expect("simulation starts");
    assert_eq!(simulation.status, RunStatus::Running);

    let result = simulation.submit_action(ActionRequest {
        request_id: "unauthorized-hvac".to_string(),
        agent_id: "cockpit-agent".to_string(),
        target: "hvac-1".to_string(),
        capability_id: "climate.restoreComfort".to_string(),
        expected_state_version: simulation.snapshot.version,
        expires_at_tick: simulation.snapshot.tick + 1,
        correlation_id: "unauthorized-hvac-corr".to_string(),
    });

    assert_eq!(result.status, ActionStatus::Rejected);
    assert_eq!(result.error_code, Some(ErrorCode::CapabilityDenied));
}

#[test]
fn repeated_domain_action_is_rejected_without_reapplying_the_effect() {
    let scenario =
        load_scenario("scenarios/heatwave-thermal-comfort.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("duplicate-domain-action", scenario);
    simulation.start().expect("simulation starts");

    let first = simulation.submit_action(ActionRequest {
        request_id: "first-hvac".to_string(),
        agent_id: "cockpit-agent".to_string(),
        target: "hvac-1".to_string(),
        capability_id: "climate.restoreComfort".to_string(),
        expected_state_version: simulation.snapshot.version,
        expires_at_tick: simulation.snapshot.tick + 1,
        correlation_id: "first-hvac-corr".to_string(),
    });
    assert_eq!(first.status, ActionStatus::Applied);
    simulation
        .step_without_agent()
        .expect("first action commits");
    assert_eq!(simulation.snapshot.environment.temperature_c, 25.5);

    let second = simulation.submit_action(ActionRequest {
        request_id: "second-hvac".to_string(),
        agent_id: "cockpit-agent".to_string(),
        target: "hvac-1".to_string(),
        capability_id: "climate.restoreComfort".to_string(),
        expected_state_version: simulation.snapshot.version,
        expires_at_tick: simulation.snapshot.tick + 1,
        correlation_id: "second-hvac-corr".to_string(),
    });
    assert_eq!(second.status, ActionStatus::Rejected);
    assert_eq!(second.error_code, Some(ErrorCode::PreconditionFailed));
    assert_eq!(simulation.snapshot.environment.temperature_c, 25.5);
}
