use std::time::{Duration, Instant};

use cockpit_agent::{LocalMcpServer, RuleAgent};
use cockpit_scenario::load_scenario;
use cockpit_world::{EventEnvelope, EventPayload, Simulation};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub scenario_path: String,
    pub ticks: u64,
    pub active_entities: u64,
    pub events_per_minute: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReport {
    pub scenario_id: String,
    pub scenario_hash: String,
    pub seed: u64,
    pub ticks: u64,
    pub active_entities: u64,
    pub events_per_minute: u64,
    pub average_tick_ms: f64,
    pub p50_tick_ms: f64,
    pub p95_tick_ms: f64,
    pub p99_tick_ms: f64,
    pub peak_tick_ms: f64,
    pub recording_bytes: usize,
    pub synthetic_event_count: u64,
    pub synthetic_workload_hash: String,
    /// Peak resident set size in bytes, when the platform exposes it without
    /// extra dependencies; `None` means it was not captured on this OS.
    pub peak_memory_bytes: Option<u64>,
    /// How `peak_memory_bytes` was obtained (or why it is absent).
    pub peak_memory_source: String,
    /// Target triple the benchmark ran on, for cross-platform acceptance.
    pub target: String,
}

pub fn run(config: BenchmarkConfig) -> anyhow::Result<BenchmarkReport> {
    let scenario = load_scenario(&config.scenario_path)?;
    let mut samples = Vec::with_capacity(config.ticks as usize);
    let mut workload_hasher = Sha256::new();
    let mut synthetic_event_count = 0_u64;
    let mut simulation = Simulation::new("benchmark-run", scenario.clone());
    simulation.start()?;
    let mut agent = RuleAgent::default();
    let mut server = LocalMcpServer::default();
    let mut recording = cockpit_recording::Recording::new("benchmark-run", &scenario);

    for _ in 0..config.ticks {
        let tick_started = Instant::now();
        let mut step = agent.step(&mut simulation, &mut server)?;
        let synthetic_events = synthetic_event_work(
            simulation.snapshot.tick,
            config.active_entities,
            config.events_per_minute,
        );
        for event in synthetic_events {
            workload_hasher.update(serde_json::to_vec(&event)?);
            step.events.push(event);
            synthetic_event_count += 1;
        }
        let elapsed = tick_started.elapsed();
        samples.push(elapsed);
        recording.push(step);
    }
    let mut nanos: Vec<u128> = samples.iter().map(Duration::as_nanos).collect();
    nanos.sort_unstable();
    let average_tick_ms = nanos.iter().sum::<u128>() as f64 / nanos.len() as f64 / 1_000_000.0;
    let percentile = |percent: usize| -> f64 {
        let index = ((nanos.len() - 1) * percent / 100).min(nanos.len() - 1);
        nanos[index] as f64 / 1_000_000.0
    };
    let recording_bytes = serde_json::to_vec(&recording)?.len();
    let peak_memory_bytes = crate::memory::peak_resident_bytes();

    Ok(BenchmarkReport {
        scenario_id: scenario.id,
        scenario_hash: scenario.scenario_hash,
        seed: scenario.seed,
        ticks: config.ticks,
        active_entities: config.active_entities,
        events_per_minute: config.events_per_minute,
        average_tick_ms,
        p50_tick_ms: percentile(50),
        p95_tick_ms: percentile(95),
        p99_tick_ms: percentile(99),
        peak_tick_ms: nanos.last().copied().unwrap_or_default() as f64 / 1_000_000.0,
        recording_bytes,
        synthetic_event_count,
        synthetic_workload_hash: format!("sha256:{:x}", workload_hasher.finalize()),
        peak_memory_bytes,
        peak_memory_source: crate::memory::peak_memory_source().to_string(),
        target: option_env!("COCKPIT_TARGET")
            .unwrap_or("unknown-target")
            .to_string(),
    })
}

fn synthetic_event_work(
    tick: u64,
    active_entities: u64,
    events_per_minute: u64,
) -> Vec<EventEnvelope> {
    let events_this_tick = (events_per_minute / 60).max(1);
    let mut events = Vec::with_capacity(events_this_tick as usize);
    for sequence in 0..events_this_tick {
        let entity = (tick.wrapping_mul(events_this_tick) + sequence) % active_entities.max(1);
        events.push(EventEnvelope {
            event_id: format!("benchmark-{tick}-{sequence}"),
            event_type: "SyntheticWorkloadEvent".to_string(),
            run_id: "benchmark-run".to_string(),
            tick,
            source: "benchmark".to_string(),
            priority: 0,
            sequence,
            correlation_id: format!("benchmark-{tick}"),
            payload: EventPayload {
                message: "synthetic capacity workload".to_string(),
                target: Some(format!("entity-{entity}")),
                value: Some(entity as f64),
                error_code: None,
            },
        });
    }
    events
}
