//! Resource-driven capability catalog.
//!
//! Every action a scenario agent may perform is defined in `capabilities.yaml`
//! (loaded once at runtime startup) rather than hardcoded as a Rust enum
//! variant. A [`CapabilityDefinition`] carries everything the previous
//! `Command` enum plus `DomainEffectResolver` match arm used to hardcode:
//! wire name, target entity, write set (for conflict arbitration), the
//! concrete field writes an applied action performs, and the events it
//! emits. Adding a new scenario capability requires only a new catalog entry
//! and a scenario YAML reference -- no Rust code changes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::world::{DeviceLifecycle, WorldSnapshot};

/// A fixed entity-id placeholder resolved against the action request and the
/// live world snapshot at effect-application time. Any catalog entity id that
/// does not match one of these is treated as a literal, scenario-independent
/// entity id (e.g. `"cabin"`, `"hvac-1"`), which is correct for shared world
/// singletons present in every scenario.
const PLACEHOLDER_TARGET: &str = "$target";
const PLACEHOLDER_ACTOR: &str = "$actor";
const PLACEHOLDER_PRIMARY_HUMAN: &str = "$primaryHuman";

/// One field write an applied capability performs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityOperation {
    pub entity_id: String,
    pub path: String,
    pub value: JsonValue,
}

/// One event an applied capability emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub value: Option<f64>,
    pub message: String,
}

/// A single catalog-defined capability, equivalent to one former `Command`
/// enum variant plus its `DomainEffectResolver` match arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDefinition {
    pub id: String,
    pub wire_name: String,
    pub target_id: String,
    pub write_set: Vec<String>,
    pub resolver: String,
    /// When present, the action's target device must declare this string in
    /// its `capabilities` list (mirrors the previous `EngineShutdown`-only
    /// capability-gate check).
    #[serde(default)]
    pub requires_capability: Option<String>,
    pub operations: Vec<CapabilityOperation>,
    #[serde(default)]
    pub events: Vec<CapabilityEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityCatalogDocument {
    #[allow(dead_code)]
    schema_version: u32,
    capabilities: Vec<CapabilityDefinition>,
}

/// The loaded, indexed capability catalog. Built once at runtime startup and
/// shared (by reference or clone) across every scenario.
#[derive(Debug, Clone, Default)]
pub struct CapabilityCatalog {
    by_id: BTreeMap<String, CapabilityDefinition>,
    by_wire_name: BTreeMap<String, String>,
}

/// Bytes of the shipped capability catalog, embedded at compile time so every
/// binary and test has a working catalog without a runtime file lookup.
pub const DEFAULT_CAPABILITIES_YAML: &str = include_str!("../../../capabilities.yaml");

impl CapabilityCatalog {
    /// Parse a capability catalog document from YAML bytes.
    pub fn from_yaml(bytes: &str) -> Result<Self, String> {
        let document: CapabilityCatalogDocument = serde_yaml::from_str(bytes)
            .map_err(|error| format!("invalid capability catalog YAML: {error}"))?;
        let mut by_id = BTreeMap::new();
        let mut by_wire_name = BTreeMap::new();
        for capability in document.capabilities {
            if by_id.contains_key(&capability.id) {
                return Err(format!("duplicate capability id '{}'", capability.id));
            }
            if by_wire_name.contains_key(&capability.wire_name) {
                return Err(format!(
                    "duplicate capability wireName '{}'",
                    capability.wire_name
                ));
            }
            by_wire_name.insert(capability.wire_name.clone(), capability.id.clone());
            by_id.insert(capability.id.clone(), capability);
        }
        Ok(Self {
            by_id,
            by_wire_name,
        })
    }

    /// Load the catalog embedded in the binary at compile time. This is the
    /// standard way every runtime and test obtains a catalog.
    pub fn load_default() -> Self {
        Self::from_yaml(DEFAULT_CAPABILITIES_YAML)
            .expect("the embedded default capability catalog must always parse")
    }

    pub fn get(&self, id: &str) -> Option<&CapabilityDefinition> {
        self.by_id.get(id)
    }

    pub fn get_by_wire_name(&self, wire_name: &str) -> Option<&CapabilityDefinition> {
        self.by_wire_name
            .get(wire_name)
            .and_then(|id| self.by_id.get(id))
    }

    /// All capability ids known to the catalog, in stable (sorted) order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.by_id.keys().map(String::as_str)
    }

    /// All capability definitions, in stable (id-sorted) order.
    pub fn definitions(&self) -> impl Iterator<Item = &CapabilityDefinition> {
        self.by_id.values()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.by_id.contains_key(id)
    }
}

/// Resolve a catalog entity-id placeholder (or literal id) against an action
/// request and the live snapshot.
pub fn resolve_entity_id<'a>(
    placeholder: &'a str,
    snapshot: &WorldSnapshot,
    target: &'a str,
    actor_agent_id: &str,
) -> Option<String> {
    match placeholder {
        PLACEHOLDER_TARGET => Some(target.to_string()),
        PLACEHOLDER_ACTOR => snapshot.human(actor_agent_id).map(|human| human.id.clone()),
        PLACEHOLDER_PRIMARY_HUMAN => snapshot.primary_human().map(|human| human.id.clone()),
        literal => Some(literal.to_string()),
    }
}

/// Every `(entityIdRole, path)` pair this module knows how to read/write.
/// Kept for documentation and for scenario-side validation of influence
/// rule component paths; the actual dispatch lives in `read_field`/`write_field`.
pub const KNOWN_COMPONENT_PATH_SUFFIXES: &[&str] = &[
    "shutdown",
    "lifecycle",
    "environment.fireActive",
    "environment.smokeDensity",
    "environment.visibility",
    "environment.temperatureC",
    "environment.zonesTemperatureC",
    "environment.noiseDb",
    "active",
    "volumeDb",
    "cockpitSystems.climate.comfortTargetC",
    "cockpitSystems.climate.coolingActive",
    "cockpitSystems.climate.seatVentilationActive",
    "cockpitSystems.climate.defogActive",
    "cockpitSystems.driverAssistance.fatigueInterventionActive",
    "cockpitSystems.driverAssistance.takeoverAcknowledged",
    "cockpitSystems.driverAssistance.takeoverHmiActive",
    "cockpitSystems.occupantCare.childProtectionActive",
    "cockpitSystems.occupantCare.medicalResponseActive",
    "cockpitSystems.occupantCare.emergencyContacted",
    "cockpitSystems.occupantCare.guardianNotified",
    "cockpitSystems.occupantCare.remoteUnlockRequested",
    "cockpitSystems.experience.privacyModeActive",
    "cockpitSystems.experience.chargingPlanAccepted",
    "cockpitSystems.experience.mediaSessionsIsolated",
    "cockpitSystems.experience.occupantProfilesIsolated",
    "cockpitSystems.mobility.emergencyRouteActive",
    "cockpitSystems.mobility.chargingRouteActive",
    "cockpitSystems.mobility.chargerServiceConnected",
    "cockpitSystems.connectivity.emergencyCallActive",
    "cockpitSystems.connectivity.remoteServicesIsolated",
    "cockpitSystems.connectivity.trustedLocalAlertActive",
    "cockpitSystems.cybersecurity.safeModeActive",
    "cockpitSystems.cybersecurity.networkIsolated",
    "cockpitSystems.cybersecurity.identityVerified",
    "pilot.stress",
    "pilot.attention",
];

/// Apply one catalog-defined field write to the world snapshot. `entity_id`
/// must already be a resolved, literal entity id (placeholders resolved by
/// the caller via [`resolve_entity_id`]).
///
/// Device-scoped paths (`shutdown`, `lifecycle`, `cockpitSystems.*` written
/// through a device target) and cabin/human-scoped paths (`environment.*`,
/// `pilot.*`) are dispatched by entity id first: `"cabin"` always addresses
/// the cabin environment, human ids address the matching human, and every
/// other id is treated as a device. `cockpitSystems.*` operations mutate the
/// simulation-wide `cockpit_systems` bucket regardless of which device
/// entity id names the acting system, mirroring the previous hardcoded
/// `EffectOp` variants that ignored their nominal target for that bucket.
pub fn write_field(
    snapshot: &mut WorldSnapshot,
    entity_id: &str,
    path: &str,
    value: &JsonValue,
) -> Result<(), String> {
    if let Some(suffix) = path.strip_prefix("cockpitSystems.") {
        return write_cockpit_systems_field(snapshot, suffix, value);
    }
    if entity_id == "cabin" {
        return write_cabin_field(snapshot, path, value);
    }
    if entity_id == "alarm-1" {
        return write_alarm_field(snapshot, path, value);
    }
    if let Some(human) = snapshot.human_mut(entity_id) {
        return write_human_field(human, path, value);
    }
    if let Some(device) = snapshot.device_mut(entity_id) {
        return write_device_field(device, path, value);
    }
    Err(format!(
        "unknown entity '{entity_id}' for capability field write '{path}'"
    ))
}

fn write_alarm_field(
    snapshot: &mut WorldSnapshot,
    path: &str,
    value: &JsonValue,
) -> Result<(), String> {
    match path {
        "active" => {
            snapshot.alarm.active = value
                .as_bool()
                .ok_or_else(|| format!("expected boolean value for alarm {path}"))?
        }
        "volumeDb" => {
            snapshot.alarm.volume_db = value
                .as_f64()
                .ok_or_else(|| format!("expected numeric value for alarm {path}"))?
        }
        other => return Err(format!("unknown alarm field '{other}'")),
    }
    Ok(())
}

fn write_cockpit_systems_field(
    snapshot: &mut WorldSnapshot,
    suffix: &str,
    value: &JsonValue,
) -> Result<(), String> {
    let systems = &mut snapshot.cockpit_systems;
    let as_bool = || {
        value
            .as_bool()
            .ok_or_else(|| format!("expected boolean value for cockpitSystems.{suffix}"))
    };
    let as_f64 = || {
        value
            .as_f64()
            .ok_or_else(|| format!("expected numeric value for cockpitSystems.{suffix}"))
    };
    match suffix {
        "climate.comfortTargetC" => systems.climate.comfort_target_c = Some(as_f64()?),
        "climate.coolingActive" => systems.climate.cooling_active = as_bool()?,
        "climate.seatVentilationActive" => systems.climate.seat_ventilation_active = as_bool()?,
        "climate.defogActive" => systems.climate.defog_active = as_bool()?,
        "driverAssistance.fatigueInterventionActive" => {
            systems.driver_assistance.fatigue_intervention_active = as_bool()?
        }
        "driverAssistance.takeoverAcknowledged" => {
            systems.driver_assistance.takeover_acknowledged = as_bool()?
        }
        "driverAssistance.takeoverHmiActive" => {
            systems.driver_assistance.takeover_hmi_active = as_bool()?
        }
        "occupantCare.childProtectionActive" => {
            systems.occupant_care.child_protection_active = as_bool()?
        }
        "occupantCare.medicalResponseActive" => {
            systems.occupant_care.medical_response_active = as_bool()?
        }
        "occupantCare.emergencyContacted" => systems.occupant_care.emergency_contacted = as_bool()?,
        "occupantCare.guardianNotified" => systems.occupant_care.guardian_notified = as_bool()?,
        "occupantCare.remoteUnlockRequested" => {
            systems.occupant_care.remote_unlock_requested = as_bool()?
        }
        "experience.privacyModeActive" => systems.experience.privacy_mode_active = as_bool()?,
        "experience.chargingPlanAccepted" => systems.experience.charging_plan_accepted = as_bool()?,
        "experience.mediaSessionsIsolated" => {
            systems.experience.media_sessions_isolated = as_bool()?
        }
        "experience.occupantProfilesIsolated" => {
            systems.experience.occupant_profiles_isolated = as_bool()?
        }
        "mobility.emergencyRouteActive" => systems.mobility.emergency_route_active = as_bool()?,
        "mobility.chargingRouteActive" => systems.mobility.charging_route_active = as_bool()?,
        "mobility.chargerServiceConnected" => {
            systems.mobility.charger_service_connected = as_bool()?
        }
        "connectivity.emergencyCallActive" => {
            systems.connectivity.emergency_call_active = as_bool()?
        }
        "connectivity.remoteServicesIsolated" => {
            systems.connectivity.remote_services_isolated = as_bool()?
        }
        "connectivity.trustedLocalAlertActive" => {
            systems.connectivity.trusted_local_alert_active = as_bool()?
        }
        "cybersecurity.safeModeActive" => systems.cybersecurity.safe_mode_active = as_bool()?,
        "cybersecurity.networkIsolated" => systems.cybersecurity.network_isolated = as_bool()?,
        "cybersecurity.identityVerified" => systems.cybersecurity.identity_verified = as_bool()?,
        other => return Err(format!("unknown cockpitSystems field '{other}'")),
    }
    Ok(())
}

fn write_cabin_field(
    snapshot: &mut WorldSnapshot,
    path: &str,
    value: &JsonValue,
) -> Result<(), String> {
    let as_bool = || {
        value
            .as_bool()
            .ok_or_else(|| format!("expected boolean value for cabin {path}"))
    };
    let as_f64 = || {
        value
            .as_f64()
            .ok_or_else(|| format!("expected numeric value for cabin {path}"))
    };
    match path {
        "environment.fireActive" => snapshot.environment.fire_active = as_bool()?,
        "environment.smokeDensity" => snapshot.environment.smoke_density = as_f64()?,
        "environment.visibility" => snapshot.environment.visibility = as_f64()?,
        "environment.noiseDb" => snapshot.environment.noise_db = as_f64()?,
        "environment.temperatureC" => {
            let target = as_f64()?;
            snapshot.environment.temperature_c = target;
        }
        "environment.zonesTemperatureC" => {
            let target = as_f64()?;
            for zone in snapshot.environment.zones.values_mut() {
                zone.temperature_c = target;
            }
        }
        other => return Err(format!("unknown cabin field '{other}'")),
    }
    Ok(())
}

fn write_human_field(
    human: &mut crate::world::HumanState,
    path: &str,
    value: &JsonValue,
) -> Result<(), String> {
    let as_f64 = || {
        value
            .as_f64()
            .ok_or_else(|| format!("expected numeric value for human {path}"))
    };
    match path {
        "pilot.stress" => human.stress = as_f64()?,
        "pilot.attention" => human.attention = as_f64()?,
        other => return Err(format!("unknown human field '{other}'")),
    }
    Ok(())
}

fn write_device_field(
    device: &mut crate::world::DeviceState,
    path: &str,
    value: &JsonValue,
) -> Result<(), String> {
    match path {
        "shutdown" => {
            device.shutdown = value
                .as_bool()
                .ok_or_else(|| format!("expected boolean value for device {path}"))?
        }
        "lifecycle" => {
            let lifecycle = value
                .as_str()
                .ok_or_else(|| format!("expected string value for device {path}"))?;
            device.lifecycle = match lifecycle {
                "normal" => DeviceLifecycle::Normal,
                "warning" => DeviceLifecycle::Warning,
                "failed" => DeviceLifecycle::Failed,
                "recovering" => DeviceLifecycle::Recovering,
                other => return Err(format!("unknown device lifecycle value '{other}'")),
            };
        }
        "engine.health" => {
            device.health = value
                .as_f64()
                .ok_or_else(|| format!("expected numeric value for device {path}"))?
        }
        other => return Err(format!("unknown device field '{other}'")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_loads_and_contains_every_migrated_capability() {
        let catalog = CapabilityCatalog::load_default();
        let expected_ids = [
            "engine.shutdown",
            "alarm.activate",
            "climate.restoreComfort",
            "visibility.activateDefog",
            "driver.activateFatigueIntervention",
            "occupant.activateChildProtection",
            "health.activateMedicalResponse",
            "privacy.activateMode",
            "energy.acceptChargingPlan",
            "adas.acknowledgeTakeover",
            "cybersecurity.enterSafeMode",
        ];
        for id in expected_ids {
            assert!(catalog.contains(id), "missing capability {id}");
        }
        assert_eq!(catalog.ids().count(), expected_ids.len());
    }

    #[test]
    fn wire_name_lookup_matches_id_lookup() {
        let catalog = CapabilityCatalog::load_default();
        let by_wire = catalog
            .get_by_wire_name("engineShutdown")
            .expect("engineShutdown is registered");
        assert_eq!(by_wire.id, "engine.shutdown");
    }

    #[test]
    fn duplicate_capability_id_is_rejected() {
        let yaml = r#"
schemaVersion: 1
capabilities:
  - id: a.one
    wireName: aOne
    targetId: x
    writeSet: []
    resolver: r
    operations: []
  - id: a.one
    wireName: aTwo
    targetId: x
    writeSet: []
    resolver: r
    operations: []
"#;
        assert!(CapabilityCatalog::from_yaml(yaml).is_err());
    }
}
