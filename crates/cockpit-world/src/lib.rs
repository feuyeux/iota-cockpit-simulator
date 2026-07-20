pub mod action;
pub mod capability;
pub mod clock;
pub mod digital_twin;
pub mod effects;
pub mod error;
pub mod event;
mod generated_vehicle_fire;
pub mod id;
pub mod influence;
pub mod perception;
pub mod sensor;
pub mod simulation;
pub mod world;

pub use action::{ActionRequest, ActionResult, ActionStatus, AgentGrant, ErrorCode, ScriptedAgent};
pub use capability::{
    CapabilityCatalog, CapabilityDefinition, CapabilityEvent, CapabilityOperation,
};
pub use clock::{ClockConfig, ClockMode, RunStatus};
pub use digital_twin::{
    CALIBRATION_PROFILE_ID, CALIBRATION_SOURCE_SHA256, COMBUSTION_PROFILE_ID,
    COMBUSTION_SOURCE_SHA256, CabinZoneState, CalibrationProvenance, DIGITAL_TWIN_MODEL_VERSION,
    DigitalTwinParameters, DigitalTwinStep, PhysiologyDelta, PhysiologyState,
    advance as advance_digital_twin, advance_cohb_pct, advance_two_node_temperatures,
    barometric_pressure_pa, measured_vehicle_fire_hrr_kw, smoke_removal_rate_s,
};
pub use effects::{
    ActionIntent, DeviceCapabilityResolver, EffectEvent, EffectOp, EffectPlan, IntentResolver,
    resolve_action, validate_action as validate_effect_action,
};
pub use error::{SimulationError, SimulationResult};
pub use event::{EventEnvelope, EventPayload, ToolCallTrace};
pub use influence::{
    ArbitrationOutcome, ConflictPolicy, InfluenceDecision, InfluenceOp, InfluenceRule,
    InfluenceSchedule, Subscription, arbitrate, schedule_due,
};
pub use perception::{
    compact_memory, delivered_and_pending, enqueue_physical_event, enqueue_social_event,
    perception_delay_ticks,
};
pub use sensor::{Observation, SensorQuality};
pub use simulation::{
    Fault, HumanStateDelta, PluginFailureRecord, Simulation, SimulationScenario, StateDiff,
    StepRecord,
};
pub use world::{
    AlarmState, BigFiveTraits, CabinEnvironment, ClimateControlState, CockpitSystemsState,
    ConnectivityState, CybersecurityState, DeviceLifecycle, DeviceState, DriverAssistanceState,
    DynamicEntity, ExperienceState, HumanState, MobilityState, NeedsState, OccupantCareState,
    OuterEnvironmentState, PerceivedEvent, Persona, WorldSnapshot,
};
