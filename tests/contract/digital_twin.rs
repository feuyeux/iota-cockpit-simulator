use cockpit_scenario::load_scenario;
use cockpit_world::{
    CALIBRATION_SOURCE_SHA256, COMBUSTION_PROFILE_ID, COMBUSTION_SOURCE_SHA256,
    DigitalTwinParameters, Simulation, advance_cohb_pct, advance_digital_twin,
    advance_two_node_temperatures, barometric_pressure_pa, measured_vehicle_fire_hrr_kw,
    smoke_removal_rate_s,
};

#[test]
fn runtime_profile_matches_reproducible_real_vehicle_calibration() {
    let profile = DigitalTwinParameters::default();
    assert_eq!(profile.calibration.profile_id, "mendeley-sedan-v1");
    assert_eq!(profile.calibration.source_doi, "10.17632/8mfgd8w9rg.1");
    assert_eq!(profile.calibration.source_sha256, CALIBRATION_SOURCE_SHA256);
    assert_eq!(profile.calibration.training_observations, 911);
    assert_eq!(profile.calibration.holdout_observations, 390);
    assert!(profile.calibration.accepted);
    assert!(profile.calibration.holdout_rmse_c <= 2.1);
    assert!((profile.envelope_ua_w_k - 62.263_969_659_348_46).abs() < 1e-12);
    assert!((profile.solar_gain_w_per_kw_m2 - 299.334_128_006_222_16).abs() < 1e-12);
}

#[test]
fn combustion_source_replays_hash_gated_full_scale_vehicle_measurements() {
    let profile = DigitalTwinParameters::default();
    assert_eq!(profile.combustion_profile_id, COMBUSTION_PROFILE_ID);
    assert_eq!(
        COMBUSTION_SOURCE_SHA256,
        "4957b94564cd338dca3098e849309e5ce442f3c8a5e6191375a42d92f2463a26"
    );
    assert!((profile.effective_heat_combustion_mj_kg - 36.0).abs() < f64::EPSILON);
    assert!((profile.soot_yield_kg_kg - 0.0569).abs() < f64::EPSILON);
    assert!((profile.carbon_monoxide_yield_kg_kg - 0.0590).abs() < f64::EPSILON);
    assert!((measured_vehicle_fire_hrr_kw(620.0) - 11_272.0).abs() < 1e-6);
    assert!((measured_vehicle_fire_hrr_kw(625.0) - 11_055.0).abs() < 1e-6);
    assert_eq!(measured_vehicle_fire_hrr_kw(6_180.0), 0.0);
}
#[test]
fn tick_creates_two_physical_zones_and_conserves_balances() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("digital-twin-contract", scenario);
    let step = advance_digital_twin(&mut simulation.snapshot, &simulation.scenario.physics, 0.1)
        .expect("physical step succeeds");
    assert_eq!(simulation.snapshot.environment.zones.len(), 2);
    assert!(simulation.snapshot.environment.zones.contains_key("front"));
    assert!(simulation.snapshot.environment.zones.contains_key("rear"));
    assert!(step.energy_residual_j.abs() <= 0.01);
    assert!(step.contaminant_residual_mg.abs() <= 0.001);
    assert!(simulation.snapshot.environment.pressure_pa.is_finite());
    assert!(simulation.snapshot.environment.humidity_pct.is_finite());
}

#[test]
fn fire_mass_balance_drives_optical_visibility_and_human_exposure() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("exposure-contract", scenario);
    advance_digital_twin(&mut simulation.snapshot, &simulation.scenario.physics, 0.1)
        .expect("zones initialize");
    simulation.snapshot.environment.fire_active = true;
    let before_cohb = simulation
        .snapshot
        .primary_human()
        .expect("scenario seeds one human")
        .physiology
        .carboxyhemoglobin_pct;
    let before_visibility = simulation.snapshot.environment.visibility;
    advance_digital_twin(&mut simulation.snapshot, &simulation.scenario.physics, 60.0)
        .expect("exposure step succeeds");
    assert!((simulation.snapshot.environment.fire_age_s - 60.0).abs() < 1e-9);
    assert!(simulation.snapshot.environment.fire_heat_release_rate_kw > 1_000.0);
    assert!(simulation.snapshot.environment.smoke_density > 0.0);
    assert!(simulation.snapshot.environment.visibility < before_visibility);
    assert!(
        simulation
            .snapshot
            .primary_human()
            .expect("scenario seeds one human")
            .physiology
            .carboxyhemoglobin_pct
            > before_cohb
    );
    assert!(
        simulation
            .snapshot
            .primary_human()
            .expect("scenario seeds one human")
            .health
            < 1.0
    );
}

#[test]
fn field_validated_cohb_model_replaces_linear_uptake_rule() {
    let profile = DigitalTwinParameters::default();
    assert!((profile.cohb_model_activity_level - 2.0).abs() < f64::EPSILON);
    assert!((profile.cohb_model_a_min - 241.0).abs() < f64::EPSILON);
    assert!((profile.cohb_model_b_inv_mmhg - 1_421.0).abs() < f64::EPSILON);

    let exposed = advance_cohb_pct(1.0, 100.0, &profile, 60.0 * 60.0);
    assert!((exposed - 4.237_883_163_785).abs() < 1e-9);
    let recovered = advance_cohb_pct(10.0, 0.0, &profile, 171.0 * 60.0);
    assert!((recovered - 4.996_640_506_086).abs() < 1e-9);

    let mut drifted = profile;
    drifted.cohb_model_a_min = 240.0;
    assert!(drifted.validate().is_err());
}

#[test]
fn cabin_pressure_tracks_altitude_instead_of_using_a_rule_delta() {
    let scenario = load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario loads");
    let mut simulation = Simulation::new("pressure-contract", scenario);
    simulation.snapshot.outer_environment.altitude_m = 2_000.0;
    let before = simulation.snapshot.environment.pressure_pa;
    advance_digital_twin(&mut simulation.snapshot, &simulation.scenario.physics, 20.0)
        .expect("pressure step succeeds");
    assert!(simulation.snapshot.environment.pressure_pa < before);
    assert!(simulation.snapshot.environment.pressure_pa > 70_000.0);
}

#[test]
fn cabin_absolute_pressure_uses_published_land_vehicle_altitude_fit() {
    assert!((barometric_pressure_pa(0.0) - 101_360.0).abs() < 1.0);
    assert!((barometric_pressure_pa(1_000.0) - 90_160.0).abs() < 1.0);
    assert!((barometric_pressure_pa(1_500.0) - 84_560.0).abs() < 1.0);
}

#[test]
fn runtime_smoke_optics_and_parked_infiltration_match_empirical_baselines() {
    let profile = DigitalTwinParameters::default();
    assert!((profile.smoke_mass_extinction_m2_mg - 0.0087).abs() < f64::EPSILON);
    assert!((0.0076..=0.0098).contains(&profile.smoke_mass_extinction_m2_mg));
    assert!((0.0..=1.4).contains(&profile.infiltration_air_changes_h));
    assert!((profile.smoke_deposition_to_air_change_ratio - 1.3).abs() < f64::EPSILON);
    let passive_air_exchange_s = profile.infiltration_air_changes_h / 3_600.0;
    let one_hour_retention =
        (-smoke_removal_rate_s(passive_air_exchange_s, &profile) * 3_600.0).exp();
    assert!((one_hour_retention - 0.562_704_868_807).abs() < 1e-9);
    assert!(profile.validate().is_ok());

    let mut invalid_smoke = profile.clone();
    invalid_smoke.smoke_mass_extinction_m2_mg = 0.000_01;
    assert!(invalid_smoke.validate().is_err());

    let mut invalid_infiltration = profile;
    invalid_infiltration.infiltration_air_changes_h = 1.5;
    assert!(invalid_infiltration.validate().is_err());
}

#[test]
fn human_heat_stress_profile_gates_resting_stability_and_humidity_direction() {
    let profile = DigitalTwinParameters::default();
    assert!((profile.evaporative_heat_transfer_ratio_per_kpa - 16.5).abs() < f64::EPSILON);
    assert!((profile.core_sweat_response_w_m2_k - 180.0).abs() < f64::EPSILON);
    assert!((profile.skin_sweat_response_w_m2_k - 20.0).abs() < f64::EPSILON);

    let simulate = |ambient_c: f64, humidity_pct: f64, metabolic_w: f64| {
        let (mut core_c, mut skin_c) = (37.0, 33.7);
        for _ in 0..3_600 {
            (core_c, skin_c) = advance_two_node_temperatures(
                core_c,
                skin_c,
                ambient_c,
                humidity_pct,
                0.0,
                metabolic_w,
                &profile,
                1.0,
            );
        }
        (core_c, skin_c)
    };

    let resting = simulate(22.0, 45.0, profile.resting_metabolic_heat_w);
    assert!((resting.0 - 37.0).abs() < 0.05);
    assert!((resting.1 - 33.7).abs() < 0.1);

    let moderate = simulate(21.2, 41.9, profile.resting_metabolic_heat_w);
    let passive_hot = simulate(39.6, 50.8, profile.resting_metabolic_heat_w);
    assert!(passive_hot.0 > moderate.0);
    assert!(passive_hot.1 > moderate.1);

    let mut previous = None;
    for humidity_pct in [23.0, 43.0, 52.0, 61.0, 71.0] {
        let endpoint = simulate(31.0, humidity_pct, 350.0);
        if let Some((previous_core, previous_skin)) = previous {
            assert!(endpoint.0 > previous_core);
            assert!(endpoint.1 > previous_skin);
        }
        previous = Some(endpoint);
    }

    let mut invalid = profile.clone();
    invalid.evaporative_heat_transfer_ratio_per_kpa = 0.0;
    assert!(invalid.validate().is_err());
}
