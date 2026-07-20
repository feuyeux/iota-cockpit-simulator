use serde::{Deserialize, Serialize};

use crate::world::WorldSnapshot;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorQuality {
    pub visibility_quality: f64,
    pub audio_quality: f64,
    pub confidence: f64,
    pub degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub observation_id: String,
    pub run_id: String,
    pub agent_id: String,
    pub sensor_id: String,
    pub observed_tick: u64,
    pub delivered_tick: u64,
    pub visible_entities: Vec<String>,
    pub alerts: Vec<String>,
    pub action_results: Vec<String>,
    pub confidence: f64,
    pub quality: SensorQuality,
}

impl Observation {
    pub fn from_snapshot(run_id: &str, agent_id: &str, snapshot: &WorldSnapshot) -> Self {
        let visibility_quality = snapshot.environment.visibility.clamp(0.0, 1.0);
        let audio_quality =
            (1.0 - ((snapshot.environment.noise_db - 45.0).max(0.0) / 55.0)).clamp(0.0, 1.0);
        let confidence = ((visibility_quality + audio_quality) / 2.0).clamp(0.0, 1.0);
        let degraded = confidence < 0.72;
        let mut alerts = Vec::new();
        if degraded && snapshot.environment.fire_active {
            alerts.push("SmokeDetected".to_string());
        }
        if snapshot.alarm.active {
            alerts.push("AlarmActive".to_string());
        }
        let has_device = |id: &str| snapshot.devices.iter().any(|device| device.id == id);
        let systems = &snapshot.cockpit_systems;
        if has_device("hvac-1")
            && snapshot.environment.temperature_c >= 35.0
            && !systems.climate.cooling_active
        {
            alerts.push("ThermalComfortRisk".to_string());
        }
        if has_device("defogger-1")
            && (snapshot.outer_environment.precipitation >= 0.8
                || snapshot.environment.visibility < 0.7)
            && !systems.climate.defog_active
        {
            alerts.push("WindshieldVisibilityRisk".to_string());
        }
        if has_device("dms-1")
            && snapshot
                .primary_human()
                .is_some_and(|human| human.attention <= 0.65)
            && !systems.driver_assistance.fatigue_intervention_active
        {
            alerts.push("DriverFatigueRisk".to_string());
        }
        if has_device("occupant-radar-1")
            && snapshot.environment.temperature_c >= 37.0
            && !systems.occupant_care.child_protection_active
        {
            alerts.push("ChildPresenceHeatRisk".to_string());
        }
        if has_device("health-sensor-1")
            && snapshot
                .human("patient-1")
                .is_some_and(|patient| patient.stress >= 0.2)
            && !systems.occupant_care.medical_response_active
        {
            alerts.push("MedicalEmergencyRisk".to_string());
        }
        if has_device("voice-array-1")
            && snapshot.humans.len() >= 4
            && !systems.experience.privacy_mode_active
        {
            alerts.push("MultiUserPrivacyConflict".to_string());
        }
        if has_device("battery-1")
            && has_device("navigation-1")
            && (snapshot.outer_environment.external_temperature_c < 0.0
                || snapshot.outer_environment.altitude_m > 1_500.0)
            && !systems.experience.charging_plan_accepted
        {
            alerts.push("EvRangeRisk".to_string());
        }
        if has_device("adas-controller-1")
            && snapshot.outer_environment.precipitation >= 0.3
            && !systems.driver_assistance.takeover_acknowledged
        {
            alerts.push("AdasTakeoverRequired".to_string());
        }
        if has_device("security-monitor-1") && !systems.cybersecurity.safe_mode_active {
            alerts.push("CyberControlAnomaly".to_string());
        }

        Self {
            observation_id: format!("{run_id}-obs-{}", snapshot.tick),
            run_id: run_id.to_string(),
            agent_id: agent_id.to_string(),
            sensor_id: "pilot-default".to_string(),
            observed_tick: snapshot.tick,
            delivered_tick: snapshot.tick,
            visible_entities: std::iter::once("cabin".to_string())
                .chain(snapshot.humans.iter().map(|human| human.id.clone()))
                .chain(snapshot.devices.iter().map(|device| device.id.clone()))
                .chain(std::iter::once("alarm-1".to_string()))
                .collect(),
            alerts,
            action_results: Vec::new(),
            confidence,
            quality: SensorQuality {
                visibility_quality,
                audio_quality,
                confidence,
                degraded,
            },
        }
    }

    /// Human turns use a subjective, location-scoped observation. Delayed
    /// physical and social events are delivered through the perception queue,
    /// not reconstructed here from the authoritative snapshot.
    pub fn for_human(run_id: &str, human_id: &str, snapshot: &WorldSnapshot) -> Self {
        let mut observation = Self::from_snapshot(run_id, human_id, snapshot);
        let Some(human) = snapshot.human(human_id) else {
            observation.visible_entities.clear();
            observation.alerts.clear();
            return observation;
        };
        observation.visible_entities = std::iter::once("cabin".to_string())
            .chain(
                snapshot
                    .humans
                    .iter()
                    .filter(|other| other.location == human.location)
                    .map(|other| other.id.clone()),
            )
            .collect();
        // A human sees only the actionable alerts associated with commands it
        // is explicitly allowed to propose. This preserves the perceived-world
        // boundary while allowing a delegated primary operator to respond.
        observation.alerts.retain(|alert| {
            let capability = match alert.as_str() {
                "SmokeDetected" => Some("engine.shutdown"),
                "ThermalComfortRisk" => Some("climate.restoreComfort"),
                "WindshieldVisibilityRisk" => Some("visibility.activateDefog"),
                "DriverFatigueRisk" => Some("driver.activateFatigueIntervention"),
                "ChildPresenceHeatRisk" => Some("occupant.activateChildProtection"),
                "MedicalEmergencyRisk" => Some("health.activateMedicalResponse"),
                "MultiUserPrivacyConflict" => Some("privacy.activateMode"),
                "EvRangeRisk" => Some("energy.acceptChargingPlan"),
                "AdasTakeoverRequired" => Some("adas.acknowledgeTakeover"),
                "CyberControlAnomaly" => Some("cybersecurity.enterSafeMode"),
                _ => None,
            };
            capability.is_some_and(|capability| {
                human
                    .action_capabilities
                    .iter()
                    .any(|granted| granted == capability)
            })
        });
        observation
    }
}

#[cfg(test)]
mod tests {
    use super::Observation;
    use crate::world::{HumanState, WorldSnapshot};

    #[test]
    fn human_observation_is_scoped_to_the_human_location() {
        let mut pilot = HumanState::new("pilot-1");
        pilot.location = "cockpit".to_string();
        let mut passenger = HumanState::new("rear-passenger-1");
        passenger.location = "rear-left".to_string();
        let snapshot = WorldSnapshot {
            run_id: "run".to_string(),
            tick: 0,
            sim_time_ms: 0,
            version: 0,
            outer_environment: Default::default(),
            environment: Default::default(),
            humans: vec![pilot, passenger],
            devices: Vec::new(),
            alarm: Default::default(),
            cockpit_systems: Default::default(),
        };

        let observation = Observation::for_human("run", "rear-passenger-1", &snapshot);
        assert_eq!(
            observation.visible_entities,
            vec!["cabin", "rear-passenger-1"]
        );
        assert!(observation.alerts.is_empty());
    }
}
