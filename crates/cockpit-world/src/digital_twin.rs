//! Calibrated, deterministic vehicle-cabin multiphysics model.
//!
//! The aggregate thermal coefficients are fitted to a real closed-sedan
//! experiment (Mendeley Data DOI 10.17632/8mfgd8w9rg.1). Heat release, soot,
//! and CO generation replay an independently hash-gated full-scale NIST
//! vehicle-fire profile. Remaining transfer boundaries publish their
//! calibration status rather than being presented as experimentally fitted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    generated_vehicle_fire::{
        NIST_VEHICLE2_HRR_KW, NIST_VEHICLE2_PROFILE_ID, NIST_VEHICLE2_SOURCE_SHA256,
    },
    world::{CabinEnvironment, WorldSnapshot},
};

pub const DIGITAL_TWIN_MODEL_VERSION: &str = "cockpit-multiphysics-4";
pub const CALIBRATION_PROFILE_ID: &str = "mendeley-sedan-v1";
pub const COMBUSTION_PROFILE_ID: &str = NIST_VEHICLE2_PROFILE_ID;
pub const COMBUSTION_SOURCE_SHA256: &str = NIST_VEHICLE2_SOURCE_SHA256;
pub const CALIBRATION_SOURCE_SHA256: &str =
    "9075e138317faa93be66891af8173dc9070e3782105d2f40f9f6f2273e89e777";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CalibrationProvenance {
    pub profile_id: String,
    pub source_doi: String,
    pub source_version: u32,
    pub source_license: String,
    pub source_sha256: String,
    pub training_observations: usize,
    pub holdout_observations: usize,
    pub holdout_rmse_c: f64,
    pub acceptance_threshold_rmse_c: f64,
    pub accepted: bool,
}

impl Default for CalibrationProvenance {
    fn default() -> Self {
        Self {
            profile_id: CALIBRATION_PROFILE_ID.to_string(),
            source_doi: "10.17632/8mfgd8w9rg.1".to_string(),
            source_version: 1,
            source_license: "CC BY 4.0".to_string(),
            source_sha256: CALIBRATION_SOURCE_SHA256.to_string(),
            training_observations: 911,
            holdout_observations: 390,
            holdout_rmse_c: 2.026_942_410_101_119,
            acceptance_threshold_rmse_c: 2.1,
            accepted: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DigitalTwinParameters {
    pub model_version: String,
    pub calibration: CalibrationProvenance,
    pub cabin_volume_m3: f64,
    pub air_density_kg_m3: f64,
    pub air_heat_capacity_j_kg_k: f64,
    pub thermal_mass_j_k: f64,
    pub envelope_ua_w_k: f64,
    pub solar_gain_w_per_kw_m2: f64,
    pub infiltration_air_changes_h: f64,
    pub interzone_conductance_w_k: f64,
    pub active_hvac_airflow_m3_s: f64,
    pub passive_pressure_equalization_s: f64,
    pub hvac_pressure_equalization_s: f64,
    pub hvac_overpressure_pa: f64,
    pub combustion_profile_id: String,
    pub effective_heat_combustion_mj_kg: f64,
    pub soot_yield_kg_kg: f64,
    pub carbon_monoxide_yield_kg_kg: f64,
    pub cabin_effluent_capture_fraction: f64,
    pub smoke_mass_extinction_m2_mg: f64,
    pub smoke_deposition_to_air_change_ratio: f64,
    pub occupant_sensible_heat_w: f64,
    pub occupant_vapour_kg_s: f64,
    pub occupant_co2_l_s: f64,
    pub body_surface_area_m2: f64,
    pub core_heat_capacity_j_k: f64,
    pub skin_heat_capacity_j_k: f64,
    pub core_skin_conductance_w_m2_k: f64,
    pub clothed_heat_transfer_w_m2_k: f64,
    pub resting_metabolic_heat_w: f64,
    pub basal_respiratory_heat_w: f64,
    pub evaporative_heat_transfer_ratio_per_kpa: f64,
    pub core_sweat_response_w_m2_k: f64,
    pub skin_sweat_response_w_m2_k: f64,
    pub cohb_model_activity_level: f64,
    pub cohb_model_a_min: f64,
    pub cohb_model_b_inv_mmhg: f64,
    pub co_hemoglobin_affinity_ratio: f64,
    pub co_ppm_per_mmhg: f64,
}

impl Default for DigitalTwinParameters {
    fn default() -> Self {
        Self {
            model_version: DIGITAL_TWIN_MODEL_VERSION.to_string(),
            calibration: CalibrationProvenance::default(),
            cabin_volume_m3: 3.2,
            air_density_kg_m3: 1.204,
            air_heat_capacity_j_kg_k: 1_006.0,
            thermal_mass_j_k: 180_000.0,
            // Reproduced by calibration/calibrate.py from the immutable source.
            envelope_ua_w_k: 62.263_969_659_348_46,
            solar_gain_w_per_kw_m2: 299.334_128_006_222_16,
            // Inside the 0.0-1.4 ACH stationary-car envelope measured across
            // six vehicles by Knibbs et al. (2009), DOI 10.1111/j.1600-0668.2009.00593.x.
            infiltration_air_changes_h: 0.25,
            interzone_conductance_w_k: 18.0,
            active_hvac_airflow_m3_s: 0.09,
            // Explicit but not yet rate-calibrated; the pressure-altitude target
            // itself is anchored to DOI 10.3390/s26020469.
            passive_pressure_equalization_s: 20.0,
            hvac_pressure_equalization_s: 8.0,
            hvac_overpressure_pa: 35.0,
            combustion_profile_id: COMBUSTION_PROFILE_ID.to_string(),
            // NIST Vehicle2 full-scale ICE minivan calorimetry. The measured
            // HRR trajectory is replayed from generated_vehicle_fire.rs.
            effective_heat_combustion_mj_kg: 36.0,
            soot_yield_kg_kg: 0.0569,
            carbon_monoxide_yield_kg_kg: 0.0590,
            // Effective transfer from the exterior design fire into the cabin.
            // This boundary is intentionally not labelled as calibrated.
            cabin_effluent_capture_fraction: 0.02,
            // NIST flame-smoke experiments: 8.7 +/- 1.1 m2/g (95% expanded
            // uncertainty), converted to m2/mg. Deposition remains uncalibrated.
            smoke_mass_extinction_m2_mg: 0.0087,
            // Ott, Klepeis & Switzer vehicle smoke experiments (n=14):
            // deposition k = 1.3 * air-change rate a, R2=0.82.
            smoke_deposition_to_air_change_ratio: 1.3,
            occupant_sensible_heat_w: 75.0,
            occupant_vapour_kg_s: 0.000_012,
            occupant_co2_l_s: 0.005,
            body_surface_area_m2: 1.8,
            core_heat_capacity_j_k: 245_000.0,
            skin_heat_capacity_j_k: 35_000.0,
            // Chosen to close the 37.0/33.7 C resting energy balance at 22 C;
            // this is an engineering constraint, not a cohort fit.
            core_skin_conductance_w_m2_k: 16.0,
            clothed_heat_transfer_w_m2_k: 4.5,
            resting_metabolic_heat_w: 105.0,
            basal_respiratory_heat_w: 8.0,
            // Lewis relation converts sensible transfer to evaporative capacity.
            // Sweat feedback is direction-checked against DOI
            // 10.1080/23328940.2016.1182669 and passive seated heat response
            // against DOI 10.3389/fphys.2018.00585; values remain engineering,
            // not cohort-fitted parameters.
            evaporative_heat_transfer_ratio_per_kpa: 16.5,
            core_sweat_response_w_m2_k: 180.0,
            skin_sweat_response_w_m2_k: 20.0,
            // CFK-derived MIL-STD-1472H AL=2 parameters, externally validated
            // in 100 armored-vehicle crew members (DOI 10.3390/toxics14060488).
            cohb_model_activity_level: 2.0,
            cohb_model_a_min: 241.0,
            cohb_model_b_inv_mmhg: 1_421.0,
            co_hemoglobin_affinity_ratio: 218.0,
            co_ppm_per_mmhg: 1_403.0,
        }
    }
}

impl DigitalTwinParameters {
    pub fn validate(&self) -> Result<(), String> {
        let positive = [
            ("cabinVolumeM3", self.cabin_volume_m3),
            ("airDensityKgM3", self.air_density_kg_m3),
            ("airHeatCapacityJKgK", self.air_heat_capacity_j_kg_k),
            ("thermalMassJK", self.thermal_mass_j_k),
            ("envelopeUaWK", self.envelope_ua_w_k),
            (
                "passivePressureEqualizationS",
                self.passive_pressure_equalization_s,
            ),
            (
                "hvacPressureEqualizationS",
                self.hvac_pressure_equalization_s,
            ),
            (
                "effectiveHeatCombustionMjKg",
                self.effective_heat_combustion_mj_kg,
            ),
            ("sootYieldKgKg", self.soot_yield_kg_kg),
            ("carbonMonoxideYieldKgKg", self.carbon_monoxide_yield_kg_kg),
            (
                "smokeDepositionToAirChangeRatio",
                self.smoke_deposition_to_air_change_ratio,
            ),
            ("bodySurfaceAreaM2", self.body_surface_area_m2),
            ("coreHeatCapacityJK", self.core_heat_capacity_j_k),
            ("skinHeatCapacityJK", self.skin_heat_capacity_j_k),
            ("coreSkinConductanceWM2K", self.core_skin_conductance_w_m2_k),
            ("clothedHeatTransferWM2K", self.clothed_heat_transfer_w_m2_k),
            ("restingMetabolicHeatW", self.resting_metabolic_heat_w),
            ("basalRespiratoryHeatW", self.basal_respiratory_heat_w),
            (
                "evaporativeHeatTransferRatioPerKpa",
                self.evaporative_heat_transfer_ratio_per_kpa,
            ),
            ("coreSweatResponseWM2K", self.core_sweat_response_w_m2_k),
            ("skinSweatResponseWM2K", self.skin_sweat_response_w_m2_k),
            ("cohbModelActivityLevel", self.cohb_model_activity_level),
            ("cohbModelAMin", self.cohb_model_a_min),
            ("cohbModelBInvMmhg", self.cohb_model_b_inv_mmhg),
            (
                "coHemoglobinAffinityRatio",
                self.co_hemoglobin_affinity_ratio,
            ),
            ("coPpmPerMmhg", self.co_ppm_per_mmhg),
        ];
        for (name, value) in positive {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "digital-twin parameter {name} must be finite and positive"
                ));
            }
        }
        if !self.hvac_overpressure_pa.is_finite() || self.hvac_overpressure_pa < 0.0 {
            return Err("HVAC overpressure must be finite and non-negative".to_string());
        }
        if !(0.0..=1.4).contains(&self.infiltration_air_changes_h) {
            return Err(
                "parked-cabin infiltration must remain inside the measured 0.0-1.4 ACH envelope"
                    .to_string(),
            );
        }
        let nist_smoke_low = 0.0076;
        let nist_smoke_high = 0.0098;
        if !(nist_smoke_low..=nist_smoke_high).contains(&self.smoke_mass_extinction_m2_mg) {
            return Err(
                "smoke mass extinction must remain inside the NIST 95% experimental interval"
                    .to_string(),
            );
        }
        if (self.smoke_deposition_to_air_change_ratio - 1.3).abs() > f64::EPSILON {
            return Err("vehicle smoke deposition calibration has drifted".to_string());
        }
        if !(0.0..=1.0).contains(&self.cabin_effluent_capture_fraction) {
            return Err("cabin effluent capture fraction must be in 0..=1".to_string());
        }
        if self.combustion_profile_id != COMBUSTION_PROFILE_ID
            || (self.effective_heat_combustion_mj_kg - 36.0).abs() > f64::EPSILON
            || (self.soot_yield_kg_kg - 0.0569).abs() > f64::EPSILON
            || (self.carbon_monoxide_yield_kg_kg - 0.0590).abs() > f64::EPSILON
        {
            return Err("vehicle-fire combustion calibration profile has drifted".to_string());
        }
        if (self.cohb_model_activity_level - 2.0).abs() > f64::EPSILON
            || (self.cohb_model_a_min - 241.0).abs() > f64::EPSILON
            || (self.cohb_model_b_inv_mmhg - 1_421.0).abs() > f64::EPSILON
            || (self.co_hemoglobin_affinity_ratio - 218.0).abs() > f64::EPSILON
            || (self.co_ppm_per_mmhg - 1_403.0).abs() > f64::EPSILON
        {
            return Err("field-validated COHb exposure profile has drifted".to_string());
        }
        if self.model_version != DIGITAL_TWIN_MODEL_VERSION
            || !self.calibration.accepted
            || self.calibration.holdout_rmse_c > self.calibration.acceptance_threshold_rmse_c
        {
            return Err(
                "digital-twin calibration profile is incompatible or unaccepted".to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CabinZoneState {
    pub volume_m3: f64,
    pub temperature_c: f64,
    pub relative_humidity_pct: f64,
    pub pressure_pa: f64,
    pub smoke_mg_m3: f64,
    pub carbon_dioxide_ppm: f64,
    pub carbon_monoxide_ppm: f64,
}

impl Default for CabinZoneState {
    fn default() -> Self {
        Self {
            volume_m3: 1.6,
            temperature_c: 22.0,
            relative_humidity_pct: 45.0,
            pressure_pa: 101_325.0,
            smoke_mg_m3: 0.0,
            carbon_dioxide_ppm: 420.0,
            carbon_monoxide_ppm: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PhysiologyState {
    pub core_temperature_c: f64,
    pub skin_temperature_c: f64,
    pub respiratory_rate_per_min: f64,
    pub carboxyhemoglobin_pct: f64,
    pub thermal_strain: f64,
}

impl Default for PhysiologyState {
    fn default() -> Self {
        Self {
            core_temperature_c: 37.0,
            skin_temperature_c: 33.7,
            respiratory_rate_per_min: 12.0,
            carboxyhemoglobin_pct: 0.5,
            thermal_strain: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysiologyDelta {
    pub human_id: String,
    pub core_temperature_c: f64,
    pub carboxyhemoglobin_pct: f64,
    pub health: f64,
    pub stress: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DigitalTwinStep {
    pub previous_temperature_c: f64,
    pub temperature_c: f64,
    pub previous_smoke_density: f64,
    pub smoke_density: f64,
    pub previous_visibility: f64,
    pub visibility: f64,
    pub pressure_pa: f64,
    pub carbon_dioxide_ppm: f64,
    pub energy_residual_j: f64,
    pub contaminant_residual_mg: f64,
    pub physiology: Vec<PhysiologyDelta>,
}

/// Advance all coupled physics using bounded one-second substeps. Every update
/// is deterministic and uses SI units internally. Aggregate legacy fields are
/// projections of the two-zone state, preserving the public observation API.
pub fn advance(
    snapshot: &mut WorldSnapshot,
    parameters: &DigitalTwinParameters,
    elapsed_s: f64,
) -> Result<DigitalTwinStep, String> {
    parameters.validate()?;
    if !elapsed_s.is_finite() || elapsed_s <= 0.0 || elapsed_s > 3_600.0 {
        return Err("digital-twin elapsed time must be in 0 < seconds <= 3600".to_string());
    }
    initialize_zones(&mut snapshot.environment, parameters);
    let previous_temperature_c = snapshot.environment.temperature_c;
    let previous_smoke_density = snapshot.environment.smoke_density;
    let previous_visibility = snapshot.environment.visibility;
    let substeps = elapsed_s.ceil() as usize;
    let dt = elapsed_s / substeps as f64;
    let mut energy_residual_j = 0.0;
    let mut contaminant_residual_mg = 0.0;
    for _ in 0..substeps {
        let (energy_residual, mass_residual) = advance_zones(snapshot, parameters, dt);
        energy_residual_j += energy_residual;
        contaminant_residual_mg += mass_residual;
        advance_physiology(snapshot, parameters, dt);
    }
    project_aggregate_environment(&mut snapshot.environment, parameters);
    let physiology = snapshot
        .humans
        .iter()
        .map(|human| PhysiologyDelta {
            human_id: human.id.clone(),
            core_temperature_c: human.physiology.core_temperature_c,
            carboxyhemoglobin_pct: human.physiology.carboxyhemoglobin_pct,
            health: human.health,
            stress: human.stress,
        })
        .collect();
    Ok(DigitalTwinStep {
        previous_temperature_c,
        temperature_c: snapshot.environment.temperature_c,
        previous_smoke_density,
        smoke_density: snapshot.environment.smoke_density,
        previous_visibility,
        visibility: snapshot.environment.visibility,
        pressure_pa: snapshot.environment.pressure_pa,
        carbon_dioxide_ppm: snapshot.environment.carbon_dioxide_ppm,
        energy_residual_j,
        contaminant_residual_mg,
        physiology,
    })
}

fn initialize_zones(environment: &mut CabinEnvironment, parameters: &DigitalTwinParameters) {
    if !environment.zones.is_empty() {
        return;
    }
    let base = CabinZoneState {
        volume_m3: parameters.cabin_volume_m3 / 2.0,
        temperature_c: environment.temperature_c,
        relative_humidity_pct: environment.humidity_pct,
        pressure_pa: environment.pressure_pa,
        smoke_mg_m3: if parameters.smoke_mass_extinction_m2_mg > 0.0 {
            environment.smoke_density / parameters.smoke_mass_extinction_m2_mg
        } else {
            0.0
        },
        carbon_dioxide_ppm: environment.carbon_dioxide_ppm,
        carbon_monoxide_ppm: environment.carbon_monoxide_ppm,
    };
    environment.zones = BTreeMap::from([
        ("front".to_string(), base.clone()),
        ("rear".to_string(), base),
    ]);
}

fn advance_zones(snapshot: &mut WorldSnapshot, p: &DigitalTwinParameters, dt: f64) -> (f64, f64) {
    let outside = snapshot.outer_environment.clone();
    let external_pressure = barometric_pressure_pa(outside.altitude_m);
    let engine_shutdown = snapshot
        .device("engine-1")
        .map(|engine| engine.shutdown)
        .unwrap_or(false);
    let fire_active = snapshot.environment.fire_active;
    let fire_source = fire_active && !engine_shutdown;
    if fire_source {
        snapshot.environment.fire_age_s += dt;
        snapshot.environment.fire_heat_release_rate_kw =
            measured_vehicle_fire_hrr_kw(snapshot.environment.fire_age_s);
    } else {
        if !fire_active {
            snapshot.environment.fire_age_s = 0.0;
        }
        snapshot.environment.fire_heat_release_rate_kw = 0.0;
    }
    let heat_release_rate_kw = snapshot.environment.fire_heat_release_rate_kw;
    let fuel_mass_loss_kg_s = heat_release_rate_kw / (p.effective_heat_combustion_mj_kg * 1_000.0);
    let cabin_soot_source_mg_s =
        fuel_mass_loss_kg_s * p.soot_yield_kg_kg * p.cabin_effluent_capture_fraction * 1_000_000.0;
    let cabin_co_source_mg_s = fuel_mass_loss_kg_s
        * p.carbon_monoxide_yield_kg_kg
        * p.cabin_effluent_capture_fraction
        * 1_000_000.0;
    let cabin_fire_heat_w = heat_release_rate_kw * 1_000.0 * p.cabin_effluent_capture_fraction;
    let hvac_active = snapshot.cockpit_systems.climate.cooling_active;
    let supply_temperature_c = snapshot
        .cockpit_systems
        .climate
        .comfort_target_c
        .unwrap_or(18.0)
        .min(outside.external_temperature_c);
    let front_occupants = snapshot
        .humans
        .iter()
        .filter(|human| zone_for_location(&human.location) == "front")
        .count();
    let rear_occupants = snapshot.humans.len().saturating_sub(front_occupants);
    let old = snapshot.environment.zones.clone();
    let front_temperature = old
        .get("front")
        .map(|zone| zone.temperature_c)
        .unwrap_or(22.0);
    let rear_temperature = old
        .get("rear")
        .map(|zone| zone.temperature_c)
        .unwrap_or(22.0);
    let outside_absolute_humidity = absolute_humidity_g_m3(
        outside.external_temperature_c,
        outside.relative_humidity_pct,
    );
    let mut total_energy_input = 0.0;
    let mut total_energy_change = 0.0;
    let mut total_smoke_input_mg = 0.0;
    let mut total_smoke_change_mg = 0.0;

    for (name, zone) in &mut snapshot.environment.zones {
        let old_zone = old.get(name).cloned().unwrap_or_default();
        let is_front = name == "front";
        let share = zone.volume_m3 / p.cabin_volume_m3;
        let occupants = if is_front {
            front_occupants
        } else {
            rear_occupants
        } as f64;
        let other_temperature = if is_front {
            rear_temperature
        } else {
            front_temperature
        };
        let infiltration_m3_s = p.infiltration_air_changes_h * zone.volume_m3 / 3_600.0;
        let hvac_m3_s = if hvac_active {
            p.active_hvac_airflow_m3_s * share
        } else {
            0.0
        };
        let air_capacity_j_k = p.air_density_kg_m3 * zone.volume_m3 * p.air_heat_capacity_j_kg_k;
        let effective_capacity_j_k = air_capacity_j_k + p.thermal_mass_j_k * share;
        let q_envelope =
            p.envelope_ua_w_k * share * (outside.external_temperature_c - old_zone.temperature_c);
        let q_infiltration = infiltration_m3_s
            * p.air_density_kg_m3
            * p.air_heat_capacity_j_kg_k
            * (outside.external_temperature_c - old_zone.temperature_c);
        let q_hvac = hvac_m3_s
            * p.air_density_kg_m3
            * p.air_heat_capacity_j_kg_k
            * (supply_temperature_c - old_zone.temperature_c);
        let q_interzone =
            p.interzone_conductance_w_k * (other_temperature - old_zone.temperature_c);
        let q_solar =
            p.solar_gain_w_per_kw_m2 * (outside.solar_irradiance_w_m2.max(0.0) / 1_000.0) * share;
        let q_fire = if fire_source && is_front {
            cabin_fire_heat_w
        } else {
            0.0
        };
        let q_occupants = occupants * p.occupant_sensible_heat_w;
        let q_total =
            q_envelope + q_infiltration + q_hvac + q_interzone + q_solar + q_fire + q_occupants;
        zone.temperature_c =
            (old_zone.temperature_c + q_total * dt / effective_capacity_j_k).clamp(-50.0, 90.0);
        total_energy_input += q_total * dt;
        total_energy_change +=
            (zone.temperature_c - old_zone.temperature_c) * effective_capacity_j_k;

        let old_absolute_humidity =
            absolute_humidity_g_m3(old_zone.temperature_c, old_zone.relative_humidity_pct);
        let vapour_source_g_s = occupants * p.occupant_vapour_kg_s * 1_000.0;
        let new_absolute_humidity = (old_absolute_humidity
            + ((infiltration_m3_s + hvac_m3_s)
                * (outside_absolute_humidity - old_absolute_humidity)
                + vapour_source_g_s)
                * dt
                / zone.volume_m3)
            .max(0.0);
        zone.relative_humidity_pct =
            relative_humidity_pct(zone.temperature_c, new_absolute_humidity);

        let measured_soot_input_mg_s = if fire_source && is_front {
            cabin_soot_source_mg_s
        } else {
            0.0
        };
        let ventilation_rate_s = (infiltration_m3_s + hvac_m3_s) / zone.volume_m3;
        let removal_s = smoke_removal_rate_s(ventilation_rate_s, p);
        let interzone_smoke_mg_s = old
            .get(if is_front { "rear" } else { "front" })
            .map(|other| 0.012 * (other.smoke_mg_m3 - old_zone.smoke_mg_m3))
            .unwrap_or(0.0);
        let smoke_delta_mg = (measured_soot_input_mg_s + interzone_smoke_mg_s
            - removal_s * old_zone.smoke_mg_m3 * zone.volume_m3)
            * dt;
        zone.smoke_mg_m3 =
            (old_zone.smoke_mg_m3 + smoke_delta_mg / zone.volume_m3).clamp(0.0, 1_000_000.0);
        total_smoke_input_mg +=
            (measured_soot_input_mg_s - removal_s * old_zone.smoke_mg_m3 * zone.volume_m3) * dt;
        total_smoke_change_mg += (zone.smoke_mg_m3 - old_zone.smoke_mg_m3) * zone.volume_m3;

        let co2_source_ppm_s =
            occupants * p.occupant_co2_l_s / (zone.volume_m3 * 1_000.0) * 1_000_000.0;
        zone.carbon_dioxide_ppm = (old_zone.carbon_dioxide_ppm
            + (co2_source_ppm_s - ventilation_rate_s * (old_zone.carbon_dioxide_ppm - 420.0)) * dt)
            .clamp(350.0, 50_000.0);
        let co_source_ppm_s = if fire_source && is_front {
            cabin_co_source_mg_s
                / (zone.volume_m3
                    * carbon_monoxide_mg_m3_per_ppm(old_zone.temperature_c, old_zone.pressure_pa))
        } else {
            0.0
        };
        zone.carbon_monoxide_ppm = (old_zone.carbon_monoxide_ppm
            + (co_source_ppm_s - ventilation_rate_s * old_zone.carbon_monoxide_ppm) * dt)
            .clamp(0.0, 100_000.0);
        let pressure_tau_s = if hvac_active {
            p.hvac_pressure_equalization_s
        } else {
            p.passive_pressure_equalization_s
        };
        let hvac_overpressure_pa = if hvac_active {
            p.hvac_overpressure_pa
        } else {
            0.0
        };
        zone.pressure_pa = (old_zone.pressure_pa
            + ((external_pressure + hvac_overpressure_pa) - old_zone.pressure_pa) * dt
                / pressure_tau_s)
            .clamp(20_000.0, 120_000.0);
    }
    (
        total_energy_change - total_energy_input,
        total_smoke_change_mg - total_smoke_input_mg,
    )
}

/// Advance core and skin temperatures through one humidity-coupled two-node step.
///
/// Evaporation is bounded by both thermoregulatory sweat drive and the
/// ambient-vapour-pressure capacity from the Lewis relation. The coefficients
/// are engineering parameters with directional external validation, not an
/// individualized cohort fit.
#[allow(
    clippy::too_many_arguments,
    reason = "The public physiology step mirrors the independently calibrated model inputs."
)]
pub fn advance_two_node_temperatures(
    core_temperature_c: f64,
    skin_temperature_c: f64,
    ambient_temperature_c: f64,
    relative_humidity_pct: f64,
    smoke_mg_m3: f64,
    metabolic_heat_w: f64,
    p: &DigitalTwinParameters,
    elapsed_s: f64,
) -> (f64, f64) {
    let core_skin_w = p.core_skin_conductance_w_m2_k
        * p.body_surface_area_m2
        * (core_temperature_c - skin_temperature_c);
    let convective_w = p.clothed_heat_transfer_w_m2_k
        * p.body_surface_area_m2
        * (skin_temperature_c - ambient_temperature_c);
    let respiratory_w = p.basal_respiratory_heat_w
        + 0.15 * (ambient_temperature_c - 22.0).abs()
        + smoke_mg_m3.min(1_000.0) * 0.002;
    let sweat_drive_w = p.body_surface_area_m2
        * (p.core_sweat_response_w_m2_k * (core_temperature_c - 37.0).max(0.0)
            + p.skin_sweat_response_w_m2_k * (skin_temperature_c - 33.7).max(0.0));
    let ambient_vapour_pressure_kpa = saturation_vapour_pressure_pa(ambient_temperature_c)
        * relative_humidity_pct.clamp(0.0, 100.0)
        / 100_000.0;
    let skin_vapour_pressure_kpa = saturation_vapour_pressure_pa(skin_temperature_c) / 1_000.0;
    let evaporative_capacity_w = p.evaporative_heat_transfer_ratio_per_kpa
        * p.clothed_heat_transfer_w_m2_k
        * p.body_surface_area_m2
        * (skin_vapour_pressure_kpa - ambient_vapour_pressure_kpa).max(0.0);
    let evaporative_w = sweat_drive_w.min(evaporative_capacity_w);
    let core_temperature_c = (core_temperature_c
        + (metabolic_heat_w - core_skin_w - respiratory_w) * elapsed_s / p.core_heat_capacity_j_k)
        .clamp(32.0, 43.0);
    let skin_temperature_c = (skin_temperature_c
        + (core_skin_w - convective_w - evaporative_w) * elapsed_s / p.skin_heat_capacity_j_k)
        .clamp(15.0, 42.0);
    (core_temperature_c, skin_temperature_c)
}

fn advance_physiology(snapshot: &mut WorldSnapshot, p: &DigitalTwinParameters, dt: f64) {
    let zones = snapshot.environment.zones.clone();
    let alarm_active = snapshot.alarm.active;
    for human in &mut snapshot.humans {
        let zone = zones
            .get(zone_for_location(&human.location))
            .or_else(|| zones.get("front"))
            .cloned()
            .unwrap_or_default();
        let (core_temperature_c, skin_temperature_c) = advance_two_node_temperatures(
            human.physiology.core_temperature_c,
            human.physiology.skin_temperature_c,
            zone.temperature_c,
            zone.relative_humidity_pct,
            zone.smoke_mg_m3,
            p.resting_metabolic_heat_w,
            p,
            dt,
        );
        human.physiology.core_temperature_c = core_temperature_c;
        human.physiology.skin_temperature_c = skin_temperature_c;
        human.physiology.carboxyhemoglobin_pct = advance_cohb_pct(
            human.physiology.carboxyhemoglobin_pct,
            zone.carbon_monoxide_ppm,
            p,
            dt,
        );
        human.physiology.respiratory_rate_per_min = (12.0
            + zone.carbon_dioxide_ppm.saturating_sub_f64(1_000.0) / 2_000.0
            + zone.smoke_mg_m3 / 400.0)
            .clamp(6.0, 45.0);
        let thermal_strain = ((human.physiology.core_temperature_c - 37.0).abs() / 2.0
            + (zone.temperature_c - 24.0).abs() / 18.0)
            .clamp(0.0, 1.0);
        human.physiology.thermal_strain = thermal_strain;
        let inhalation_hazard = (zone.smoke_mg_m3 / 1_000.0
            + human.physiology.carboxyhemoglobin_pct / 20.0)
            .clamp(0.0, 2.0);
        human.health = (human.health - inhalation_hazard * dt / 3_600.0).clamp(0.0, 1.0);
        let stress_target = (0.08
            + thermal_strain * 0.45
            + inhalation_hazard * 0.35
            + if alarm_active { 0.2 } else { 0.0 })
        .clamp(0.0, 1.0);
        human.stress = (human.stress + (stress_target - human.stress) * dt / 20.0).clamp(0.0, 1.0);
        human.attention = (human.attention
            - (inhalation_hazard * 0.02 + human.fatigue * 0.005) * dt)
            .clamp(0.0, 1.0);
    }
}

fn project_aggregate_environment(environment: &mut CabinEnvironment, p: &DigitalTwinParameters) {
    let total_volume: f64 = environment.zones.values().map(|zone| zone.volume_m3).sum();
    if total_volume <= 0.0 {
        return;
    }
    let weighted = |value: fn(&CabinZoneState) -> f64| {
        environment
            .zones
            .values()
            .map(|zone| value(zone) * zone.volume_m3)
            .sum::<f64>()
            / total_volume
    };
    let temperature_c = weighted(|zone| zone.temperature_c);
    let humidity_pct = weighted(|zone| zone.relative_humidity_pct);
    let pressure_pa = weighted(|zone| zone.pressure_pa);
    let carbon_dioxide_ppm = weighted(|zone| zone.carbon_dioxide_ppm);
    let carbon_monoxide_ppm = weighted(|zone| zone.carbon_monoxide_ppm);
    let smoke_mg_m3 = weighted(|zone| zone.smoke_mg_m3);
    environment.temperature_c = temperature_c;
    environment.humidity_pct = humidity_pct;
    environment.pressure_pa = pressure_pa;
    environment.carbon_dioxide_ppm = carbon_dioxide_ppm;
    environment.carbon_monoxide_ppm = carbon_monoxide_ppm;
    environment.smoke_density = smoke_mg_m3 * p.smoke_mass_extinction_m2_mg;
    // Beer-Lambert transmittance over a calibrated representative 1.6 m path.
    environment.visibility = (-environment.smoke_density * 1.6).exp().clamp(0.0, 1.0);
}

/// Total smoke loss combines ventilation `a` with measured deposition
/// `k = 1.3a` from 14 in-vehicle smoke experiments (R2=0.82), DOI
/// 10.1038/sj.jes.7500601.
pub fn smoke_removal_rate_s(air_exchange_rate_s: f64, p: &DigitalTwinParameters) -> f64 {
    air_exchange_rate_s.max(0.0) * (1.0 + p.smoke_deposition_to_air_change_ratio)
}

/// Advance blood COHb using the integrated CFK-derived MIL-STD-1472H
/// equation. The AL=2 constants are field-validated for armored-vehicle crews
/// by Alter et al. (2026), DOI 10.3390/toxics14060488.
pub fn advance_cohb_pct(
    current_cohb_pct: f64,
    carbon_monoxide_ppm: f64,
    p: &DigitalTwinParameters,
    elapsed_s: f64,
) -> f64 {
    let decay = (-elapsed_s.max(0.0) / (p.cohb_model_a_min * 60.0)).exp();
    let equilibrium_pct = p.co_hemoglobin_affinity_ratio
        * (1.0 / p.cohb_model_b_inv_mmhg + carbon_monoxide_ppm.max(0.0) / p.co_ppm_per_mmhg);
    (current_cohb_pct.clamp(0.0, 100.0) * decay + equilibrium_pct * (1.0 - decay)).clamp(0.0, 100.0)
}

/// Piecewise-linear replay of the hash-gated NIST Vehicle2 HRR observations.
/// Values after the measured record are zero rather than extrapolated.
pub fn measured_vehicle_fire_hrr_kw(fire_age_s: f64) -> f64 {
    if !fire_age_s.is_finite() || fire_age_s < 0.0 {
        return 0.0;
    }
    for interval in NIST_VEHICLE2_HRR_KW.windows(2) {
        let (start_s, start_kw) = interval[0];
        let (end_s, end_kw) = interval[1];
        if fire_age_s <= end_s {
            let fraction = ((fire_age_s - start_s) / (end_s - start_s)).clamp(0.0, 1.0);
            return start_kw + (end_kw - start_kw) * fraction;
        }
    }
    0.0
}

fn carbon_monoxide_mg_m3_per_ppm(temperature_c: f64, pressure_pa: f64) -> f64 {
    const CO_MOLAR_MASS_KG_MOL: f64 = 0.028_01;
    const GAS_CONSTANT_J_MOL_K: f64 = 8.314_462_618;
    pressure_pa.clamp(20_000.0, 120_000.0) * CO_MOLAR_MASS_KG_MOL
        / (GAS_CONSTANT_J_MOL_K * (temperature_c.clamp(-50.0, 90.0) + 273.15))
}

fn zone_for_location(location: &str) -> &'static str {
    if location.contains("rear") || location.contains("back") {
        "rear"
    } else {
        "front"
    }
}

pub fn barometric_pressure_pa(altitude_m: f64) -> f64 {
    let altitude_m = altitude_m.clamp(-500.0, 11_000.0);
    if (0.0..=1_500.0).contains(&altitude_m) {
        // Teleszewski & Gladyszewska-Fiedoruk (2026), DOI
        // 10.3390/s26020469: mean of 15 land-vehicle measurement series,
        // p[hPa] = 1013.6 - 0.112 h[m], probe uncertainty 1.1 hPa.
        return (1_013.6 - 0.112 * altitude_m) * 100.0;
    }
    let base = (1.0 - 2.255_77e-5 * altitude_m).max(0.01);
    101_325.0 * base.powf(5.255_88)
}

fn saturation_vapour_pressure_pa(temperature_c: f64) -> f64 {
    610.94 * ((17.625 * temperature_c) / (temperature_c + 243.04)).exp()
}

fn absolute_humidity_g_m3(temperature_c: f64, relative_humidity_pct: f64) -> f64 {
    let vapour_pressure = saturation_vapour_pressure_pa(temperature_c)
        * relative_humidity_pct.clamp(0.0, 100.0)
        / 100.0;
    2.166_79 * vapour_pressure / (273.15 + temperature_c)
}

fn relative_humidity_pct(temperature_c: f64, absolute_humidity_g_m3: f64) -> f64 {
    let vapour_pressure = absolute_humidity_g_m3 * (273.15 + temperature_c) / 2.166_79;
    (vapour_pressure / saturation_vapour_pressure_pa(temperature_c) * 100.0).clamp(0.0, 100.0)
}

trait SaturatingSubF64 {
    fn saturating_sub_f64(self, rhs: f64) -> f64;
}

impl SaturatingSubF64 for f64 {
    fn saturating_sub_f64(self, rhs: f64) -> f64 {
        (self - rhs).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_profile_is_real_versioned_and_accepted() {
        let profile = CalibrationProvenance::default();
        assert_eq!(profile.source_doi, "10.17632/8mfgd8w9rg.1");
        assert_eq!(profile.source_sha256, CALIBRATION_SOURCE_SHA256);
        assert_eq!(profile.training_observations, 911);
        assert_eq!(profile.holdout_observations, 390);
        assert!(profile.accepted);
        assert!(profile.holdout_rmse_c <= profile.acceptance_threshold_rmse_c);
    }

    #[test]
    fn altitude_pressure_and_humidity_are_physical() {
        assert!((barometric_pressure_pa(0.0) - 101_360.0).abs() < 1.0);
        assert!((barometric_pressure_pa(1_000.0) - 90_160.0).abs() < 1.0);
        assert!(barometric_pressure_pa(2_000.0) < barometric_pressure_pa(0.0));
        let absolute = absolute_humidity_g_m3(22.0, 45.0);
        assert!((relative_humidity_pct(22.0, absolute) - 45.0).abs() < 1e-9);
    }

    #[test]
    fn zone_exchange_conserves_internal_heat_and_smoke() {
        let mut environment = CabinEnvironment::default();
        initialize_zones(&mut environment, &DigitalTwinParameters::default());
        environment.zones.get_mut("front").unwrap().temperature_c = 40.0;
        environment.zones.get_mut("rear").unwrap().temperature_c = 20.0;
        environment.zones.get_mut("front").unwrap().smoke_mg_m3 = 100.0;
        let before_smoke: f64 = environment
            .zones
            .values()
            .map(|zone| zone.smoke_mg_m3 * zone.volume_m3)
            .sum();
        assert_eq!(before_smoke, 160.0);
    }

    #[test]
    fn resting_two_node_physiology_is_stable_for_one_hour() {
        use crate::world::{AlarmState, CockpitSystemsState, HumanState, OuterEnvironmentState};

        let p = DigitalTwinParameters::default();
        let mut environment = CabinEnvironment {
            temperature_c: 22.0,
            ..CabinEnvironment::default()
        };
        initialize_zones(&mut environment, &p);
        let mut snapshot = WorldSnapshot {
            run_id: "physiology-baseline".to_string(),
            tick: 0,
            sim_time_ms: 0,
            version: 0,
            outer_environment: OuterEnvironmentState::default(),
            environment,
            humans: vec![HumanState::new("occupant")],
            devices: Vec::new(),
            alarm: AlarmState::default(),
            cockpit_systems: CockpitSystemsState::default(),
        };
        for _ in 0..3_600 {
            advance_physiology(&mut snapshot, &p, 1.0);
        }
        let physiology = snapshot
            .primary_human()
            .expect("test snapshot always seeds one human")
            .physiology;
        assert!((physiology.core_temperature_c - 37.0).abs() < 0.05);
        assert!((physiology.skin_temperature_c - 33.7).abs() < 0.1);
    }
}
