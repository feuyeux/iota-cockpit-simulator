use bincode::Options;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Write};

use crate::error::{SimulationError, SimulationResult};

fn default_relative_humidity_pct() -> f64 {
    50.0
}

fn default_pressure_pa() -> f64 {
    101_325.0
}

fn default_carbon_dioxide_ppm() -> f64 {
    420.0
}

/// Environment outside the cabin (weather, altitude, external threats). Drives
/// the calibrated cabin multiphysics model; never perceived directly by humans
/// inside the cabin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OuterEnvironmentState {
    pub external_temperature_c: f64,
    #[serde(default = "default_relative_humidity_pct")]
    pub relative_humidity_pct: f64,
    #[serde(default)]
    pub solar_irradiance_w_m2: f64,
    pub altitude_m: f64,
    pub wind_speed_kmh: f64,
    pub precipitation: f64,
    pub threat_active: bool,
}

impl Default for OuterEnvironmentState {
    fn default() -> Self {
        Self {
            external_temperature_c: 20.0,
            relative_humidity_pct: 50.0,
            solar_irradiance_w_m2: 0.0,
            altitude_m: 0.0,
            wind_speed_kmh: 0.0,
            precipitation: 0.0,
            threat_active: false,
        }
    }
}

/// Aggregate cabin projection plus authoritative front/rear physical zones.
/// Existing observation clients continue to read aggregate fields; the digital
/// twin advances and conserves mass/energy in `zones` before projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CabinEnvironment {
    pub temperature_c: f64,
    pub humidity_pct: f64,
    #[serde(default = "default_pressure_pa")]
    pub pressure_pa: f64,
    #[serde(default = "default_carbon_dioxide_ppm")]
    pub carbon_dioxide_ppm: f64,
    #[serde(default)]
    pub carbon_monoxide_ppm: f64,
    pub visibility: f64,
    /// Optical extinction coefficient in m^-1, derived from conserved smoke mass.
    pub smoke_density: f64,
    pub lighting_lux: f64,
    pub noise_db: f64,
    pub fire_active: bool,
    /// Elapsed active combustion time used to index the measured NIST HRR curve.
    #[serde(default)]
    pub fire_age_s: f64,
    /// Current measured-profile heat release rate before the cabin transfer boundary.
    #[serde(default)]
    pub fire_heat_release_rate_kw: f64,
    #[serde(default)]
    pub zones: std::collections::BTreeMap<String, crate::digital_twin::CabinZoneState>,
}

impl Default for CabinEnvironment {
    fn default() -> Self {
        Self {
            temperature_c: 22.0,
            humidity_pct: 45.0,
            pressure_pa: 101_325.0,
            carbon_dioxide_ppm: 420.0,
            carbon_monoxide_ppm: 0.0,
            visibility: 1.0,
            smoke_density: 0.0,
            lighting_lux: 400.0,
            noise_db: 42.0,
            fire_active: false,
            fire_age_s: 0.0,
            fire_heat_release_rate_kw: 0.0,
            zones: std::collections::BTreeMap::new(),
        }
    }
}

/// Five-Factor ("Big Five") personality traits, each normalized to `0.0..=1.0`.
/// These are stable per-human values that anchor persona-consistent behavior
/// across ticks and are included verbatim in the backend prompt.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BigFiveTraits {
    pub openness: f64,
    pub conscientiousness: f64,
    pub extraversion: f64,
    pub agreeableness: f64,
    pub neuroticism: f64,
}

impl Default for BigFiveTraits {
    fn default() -> Self {
        Self {
            openness: 0.5,
            conscientiousness: 0.5,
            extraversion: 0.5,
            agreeableness: 0.5,
            neuroticism: 0.5,
        }
    }
}

/// Stable identity and personality description for a human entity. Rendered
/// into the per-human backend prompt alongside dynamic state (needs, memory).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub name: String,
    pub role: String,
    pub background: String,
    pub traits: BigFiveTraits,
    #[serde(default)]
    pub relationships: Vec<String>,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            name: "Pilot".to_string(),
            role: "pilot".to_string(),
            background: "Primary operator responsible for cockpit safety.".to_string(),
            traits: BigFiveTraits::default(),
            relationships: Vec::new(),
        }
    }
}

/// Dynamic needs driving intrinsic (non-event-triggered) motivation, each
/// normalized to `0.0..=1.0` where higher means the need is better satisfied.
/// Updated by deterministic rules from environment/state each tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsState {
    pub comfort: f64,
    pub safety: f64,
    pub social: f64,
}

impl Default for NeedsState {
    fn default() -> Self {
        Self {
            comfort: 1.0,
            safety: 1.0,
            social: 1.0,
        }
    }
}

/// A perceived event queued for later delivery to a human's perception
/// buffers. `available_at_tick` is computed deterministically from source
/// distance/attention at enqueue time so delivery does not require any
/// non-deterministic scheduling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerceivedEvent {
    pub origin_tick: u64,
    pub available_at_tick: u64,
    pub source: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanState {
    pub id: String,
    pub persona: Persona,
    pub needs: NeedsState,
    pub stress: f64,
    pub fatigue: f64,
    pub health: f64,
    pub attention: f64,
    #[serde(default)]
    pub physiology: crate::digital_twin::PhysiologyState,
    pub location: String,
    pub goal: String,
    /// Typed commands this human may propose. Empty means the human may not
    /// operate cockpit systems, even when another agent has those privileges.
    #[serde(default)]
    pub action_capabilities: Vec<String>,
    /// Recently perceived events (physical + social) not yet compacted into
    /// long-term memory. Ordered by `available_at_tick` ascending.
    #[serde(default)]
    pub short_term_memory: Vec<PerceivedEvent>,
    /// Compacted summaries of older short-term memory entries.
    #[serde(default)]
    pub long_term_memory: Vec<String>,
}

impl HumanState {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }
}

impl Default for HumanState {
    fn default() -> Self {
        Self {
            id: "pilot-1".to_string(),
            persona: Persona::default(),
            needs: NeedsState::default(),
            stress: 0.1,
            fatigue: 0.0,
            health: 1.0,
            attention: 0.9,
            physiology: crate::digital_twin::PhysiologyState::default(),
            location: "cockpit".to_string(),
            goal: "maintain safe cockpit state".to_string(),
            action_capabilities: Vec::new(),
            short_term_memory: Vec::new(),
            long_term_memory: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceLifecycle {
    Normal,
    Warning,
    Failed,
    Recovering,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    pub id: String,
    pub health: f64,
    pub power_state: String,
    pub lifecycle: DeviceLifecycle,
    pub faults: Vec<String>,
    pub capabilities: Vec<String>,
    pub shutdown: bool,
}

impl DeviceState {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            id: "engine-1".to_string(),
            health: 1.0,
            power_state: "powered".to_string(),
            lifecycle: DeviceLifecycle::Normal,
            faults: Vec::new(),
            capabilities: vec!["shutdown".to_string()],
            shutdown: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmState {
    pub active: bool,
    pub volume_db: f64,
}

impl Default for AlarmState {
    fn default() -> Self {
        Self {
            active: false,
            volume_db: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ClimateControlState {
    pub comfort_target_c: Option<f64>,
    pub cooling_active: bool,
    pub defog_active: bool,
    pub seat_ventilation_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DriverAssistanceState {
    pub fatigue_intervention_active: bool,
    pub takeover_acknowledged: bool,
    pub takeover_hmi_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OccupantCareState {
    pub child_protection_active: bool,
    pub medical_response_active: bool,
    pub emergency_contacted: bool,
    pub guardian_notified: bool,
    pub remote_unlock_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ExperienceState {
    pub privacy_mode_active: bool,
    pub charging_plan_accepted: bool,
    pub media_sessions_isolated: bool,
    pub occupant_profiles_isolated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MobilityState {
    pub emergency_route_active: bool,
    pub charging_route_active: bool,
    pub charger_service_connected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectivityState {
    pub emergency_call_active: bool,
    pub remote_services_isolated: bool,
    pub trusted_local_alert_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CybersecurityState {
    pub safe_mode_active: bool,
    pub network_isolated: bool,
    pub identity_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CockpitSystemsState {
    pub climate: ClimateControlState,
    pub driver_assistance: DriverAssistanceState,
    pub occupant_care: OccupantCareState,
    pub experience: ExperienceState,
    pub mobility: MobilityState,
    pub connectivity: ConnectivityState,
    pub cybersecurity: CybersecurityState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "state", rename_all = "camelCase")]
#[allow(
    clippy::large_enum_variant,
    reason = "Boxing a variant would change a widely serialized domain value and its public API."
)]
pub enum DynamicEntity {
    Human(HumanState),
    Device(DeviceState),
}

impl DynamicEntity {
    pub fn id(&self) -> &str {
        match self {
            Self::Human(human) => &human.id,
            Self::Device(device) => &device.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldSnapshot {
    pub run_id: String,
    pub tick: u64,
    pub sim_time_ms: u64,
    pub version: u64,
    pub outer_environment: OuterEnvironmentState,
    pub environment: CabinEnvironment,
    pub humans: Vec<HumanState>,
    pub devices: Vec<DeviceState>,
    pub alarm: AlarmState,
    #[serde(default)]
    pub cockpit_systems: CockpitSystemsState,
}

impl WorldSnapshot {
    /// The first human in `humans`, i.e. the scenario's primary agent-operated
    /// human. Scenario validation guarantees at least one human at load time,
    /// but dynamic entity removal (`remove_entity`) can empty `humans` at
    /// runtime, so callers must handle the `None` case rather than assume
    /// panic-free indexing.
    pub fn primary_human(&self) -> Option<&HumanState> {
        self.humans.first()
    }

    pub fn primary_human_mut(&mut self) -> Option<&mut HumanState> {
        self.humans.first_mut()
    }

    pub fn human(&self, id: &str) -> Option<&HumanState> {
        self.humans.iter().find(|human| human.id == id)
    }

    pub fn human_mut(&mut self, id: &str) -> Option<&mut HumanState> {
        self.humans.iter_mut().find(|human| human.id == id)
    }

    pub fn device(&self, id: &str) -> Option<&DeviceState> {
        self.devices.iter().find(|device| device.id == id)
    }

    pub fn device_mut(&mut self, id: &str) -> Option<&mut DeviceState> {
        self.devices.iter_mut().find(|device| device.id == id)
    }

    pub fn spawn_entity(&mut self, entity: DynamicEntity) -> Result<(), String> {
        let id = entity.id();
        if self.human(id).is_some() || self.device(id).is_some() {
            return Err(format!("entity '{id}' already exists"));
        }
        match entity {
            DynamicEntity::Human(human) => self.humans.push(human),
            DynamicEntity::Device(device) => self.devices.push(device),
        }
        Ok(())
    }

    pub fn remove_entity(&mut self, entity_id: &str) -> Option<DynamicEntity> {
        if let Some(index) = self.humans.iter().position(|human| human.id == entity_id) {
            return Some(DynamicEntity::Human(self.humans.remove(index)));
        }
        self.devices
            .iter()
            .position(|device| device.id == entity_id)
            .map(|index| DynamicEntity::Device(self.devices.remove(index)))
    }

    pub fn content_hash(&self) -> SimulationResult<String> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct HashableSnapshot<'a> {
            tick: u64,
            sim_time_ms: u64,
            version: u64,
            outer_environment: &'a OuterEnvironmentState,
            environment: &'a CabinEnvironment,
            humans: &'a [HumanState],
            devices: &'a [DeviceState],
            alarm: &'a AlarmState,
            cockpit_systems: &'a CockpitSystemsState,
        }

        let hashable = HashableSnapshot {
            tick: self.tick,
            sim_time_ms: self.sim_time_ms,
            version: self.version,
            outer_environment: &self.outer_environment,
            environment: &self.environment,
            humans: &self.humans,
            devices: &self.devices,
            alarm: &self.alarm,
            cockpit_systems: &self.cockpit_systems,
        };
        let mut hasher = Sha256::new();
        // Hash a stable binary representation directly into SHA-256. This is
        // deliberately independent of the JSON IPC representation: snapshots
        // are hashed every tick for recording/replay, so allocating and
        // escaping a complete JSON document here was a significant hot path.
        hasher.update(b"cockpit-world-snapshot-v6\\0");
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_little_endian()
            .serialize_into(HashWriter(&mut hasher), &hashable)
            .map_err(|err| SimulationError::Serialization(err.to_string()))?;
        Ok(format!("{:x}", hasher.finalize()))
    }
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
