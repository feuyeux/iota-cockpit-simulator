use serde::{Deserialize, Serialize};

use crate::{
    action::{ActionRequest, ErrorCode},
    capability::{CapabilityCatalog, CapabilityDefinition, resolve_entity_id, write_field},
    world::WorldSnapshot,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionIntent {
    pub request_id: String,
    pub actor_id: String,
    pub target_id: String,
    pub capability: String,
    pub operation: String,
    pub capability_id: String,
}

impl ActionIntent {
    pub fn from_request(request: &ActionRequest) -> Self {
        let capability = request.capability_id.clone();
        let operation = capability
            .rsplit('.')
            .next()
            .unwrap_or(capability.as_str())
            .to_string();
        Self {
            request_id: request.request_id.clone(),
            actor_id: request.agent_id.clone(),
            target_id: request.target.clone(),
            capability,
            operation,
            capability_id: request.capability_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectOp {
    pub entity_id: String,
    pub path: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectEvent {
    pub event_type: String,
    pub source: String,
    pub target: Option<String>,
    pub value: Option<f64>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectPlan {
    pub resolver: String,
    pub intent: ActionIntent,
    pub operations: Vec<EffectOp>,
    pub events: Vec<EffectEvent>,
}

impl EffectPlan {
    pub fn apply(&self, snapshot: &mut WorldSnapshot) -> Result<(), ErrorCode> {
        for operation in &self.operations {
            write_field(
                snapshot,
                &operation.entity_id,
                &operation.path,
                &operation.value,
            )
            .map_err(|_| ErrorCode::UnknownTarget)?;
        }
        Ok(())
    }
}

pub trait IntentResolver {
    fn validate(snapshot: &WorldSnapshot, intent: &ActionIntent) -> Result<(), ErrorCode>;
    fn resolve(snapshot: &WorldSnapshot, intent: &ActionIntent) -> Result<EffectPlan, ErrorCode>;
}

/// Resolves device existence, power, capability and idempotency from world
/// components, then builds the effect plan from the catalog-defined
/// capability. Scenario files never participate in successful action
/// effects; only the capability catalog and the live snapshot do.
pub struct DeviceCapabilityResolver;

impl DeviceCapabilityResolver {
    fn validate_with_catalog(
        snapshot: &WorldSnapshot,
        intent: &ActionIntent,
        capability: &CapabilityDefinition,
    ) -> Result<(), ErrorCode> {
        if capability.id == "alarm.activate" {
            return (!snapshot.alarm.active)
                .then_some(())
                .ok_or(ErrorCode::PreconditionFailed);
        }
        let device = snapshot
            .device(&intent.target_id)
            .ok_or(ErrorCode::UnknownTarget)?;
        if device.power_state != "powered" {
            return Err(ErrorCode::DeviceUnpowered);
        }
        if let Some(required) = &capability.requires_capability
            && !device.capabilities.iter().any(|owned| owned == required)
        {
            return Err(ErrorCode::CapabilityDenied);
        }
        if action_already_applied(snapshot, capability) {
            return Err(ErrorCode::PreconditionFailed);
        }
        Ok(())
    }

    fn resolve_with_catalog(
        snapshot: &WorldSnapshot,
        intent: &ActionIntent,
        capability: &CapabilityDefinition,
    ) -> Result<EffectPlan, ErrorCode> {
        Self::validate_with_catalog(snapshot, intent, capability)?;
        build_effect_plan(snapshot, intent, capability)
    }
}

fn build_effect_plan(
    snapshot: &WorldSnapshot,
    intent: &ActionIntent,
    capability: &CapabilityDefinition,
) -> Result<EffectPlan, ErrorCode> {
    let mut operations = Vec::with_capacity(capability.operations.len());
    for operation in &capability.operations {
        let entity_id = resolve_entity_id(
            &operation.entity_id,
            snapshot,
            &intent.target_id,
            &intent.actor_id,
        )
        .ok_or(ErrorCode::UnknownTarget)?;
        operations.push(EffectOp {
            entity_id,
            path: operation.path.clone(),
            value: operation.value.clone(),
        });
    }
    let mut events = Vec::with_capacity(capability.events.len());
    for event in &capability.events {
        let target = match &event.target {
            Some(target) => Some(
                resolve_entity_id(target, snapshot, &intent.target_id, &intent.actor_id)
                    .ok_or(ErrorCode::UnknownTarget)?,
            ),
            None => None,
        };
        events.push(EffectEvent {
            event_type: event.event_type.clone(),
            source: event.source.clone(),
            target,
            value: event.value,
            message: event.message.clone(),
        });
    }
    Ok(EffectPlan {
        resolver: capability.resolver.clone(),
        intent: intent.clone(),
        operations,
        events,
    })
}

/// Look up the actual authoritative system-state flag this capability's
/// idempotency depends on. Mirrors the previous per-`Command` match by
/// keying off the capability id, since idempotency is a semantic property of
/// the capability rather than something the generic operation list encodes.
fn action_already_applied(snapshot: &WorldSnapshot, capability: &CapabilityDefinition) -> bool {
    let systems = &snapshot.cockpit_systems;
    match capability.id.as_str() {
        "engine.shutdown" => snapshot
            .device(&capability.target_id)
            .is_some_and(|device| device.shutdown),
        "alarm.activate" => snapshot.alarm.active,
        "climate.restoreComfort" => systems.climate.cooling_active,
        "visibility.activateDefog" => systems.climate.defog_active,
        "driver.activateFatigueIntervention" => {
            systems.driver_assistance.fatigue_intervention_active
        }
        "occupant.activateChildProtection" => systems.occupant_care.child_protection_active,
        "health.activateMedicalResponse" => systems.occupant_care.medical_response_active,
        "privacy.activateMode" => systems.experience.privacy_mode_active,
        "energy.acceptChargingPlan" => systems.experience.charging_plan_accepted,
        "adas.acknowledgeTakeover" => systems.driver_assistance.takeover_acknowledged,
        "cybersecurity.enterSafeMode" => systems.cybersecurity.safe_mode_active,
        _ => false,
    }
}

pub fn resolve_action(
    catalog: &CapabilityCatalog,
    snapshot: &WorldSnapshot,
    request: &ActionRequest,
) -> Result<EffectPlan, ErrorCode> {
    let intent = ActionIntent::from_request(request);
    let capability = catalog
        .get(&intent.capability_id)
        .ok_or(ErrorCode::CapabilityDenied)?;
    DeviceCapabilityResolver::resolve_with_catalog(snapshot, &intent, capability)
}

pub fn validate_action(
    catalog: &CapabilityCatalog,
    snapshot: &WorldSnapshot,
    request: &ActionRequest,
) -> Result<(), ErrorCode> {
    let intent = ActionIntent::from_request(request);
    let capability = catalog
        .get(&intent.capability_id)
        .ok_or(ErrorCode::CapabilityDenied)?;
    DeviceCapabilityResolver::validate_with_catalog(snapshot, &intent, capability)
}
