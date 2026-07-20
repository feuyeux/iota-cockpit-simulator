use cockpit_simulator::benchmark::{BenchmarkConfig, run};

#[test]
fn benchmark_report_is_reproducible_and_contains_capacity_dimensions() {
    let report = run(BenchmarkConfig {
        scenario_path: "scenarios/smoke-in-cockpit.yaml".to_string(),
        ticks: 20,
        active_entities: 1_000,
        events_per_minute: 10_000,
    })
    .expect("benchmark runs");
    assert_eq!(report.seed, 42);
    assert_eq!(report.active_entities, 1_000);
    assert_eq!(report.events_per_minute, 10_000);
    assert_eq!(report.ticks, 20);
    assert!(report.p95_tick_ms >= 0.0);
    assert!(report.p99_tick_ms >= report.p95_tick_ms);
    assert!(report.recording_bytes > 0);
    assert!(
        report.recording_bytes > 20_000,
        "synthetic events are recorded"
    );
    assert_eq!(report.synthetic_event_count, 20 * 166);
    assert!(
        report.p95_tick_ms < 50.0,
        "tick p95 stays within the MVP budget"
    );
    assert!(report.synthetic_workload_hash.starts_with("sha256:"));
    // Cross-platform acceptance dimensions.
    assert_ne!(report.target, "unknown-target", "target triple is recorded");
    assert!(
        !report.peak_memory_source.is_empty(),
        "peak-memory source is always described"
    );
    // Peak memory is captured on every platform with a dependency-free source
    // (Linux /proc, libc getrusage on macOS, psapi on Windows); it is None only
    // on platforms without such a source.
    if cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows"
    )) {
        assert!(
            report
                .peak_memory_bytes
                .map(|bytes| bytes > 0)
                .unwrap_or(false),
            "platforms with a dependency-free source report a non-zero peak: source={}",
            report.peak_memory_source,
        );
        assert!(
            report.peak_memory_source.starts_with("linux:")
                || report.peak_memory_source.starts_with("macos:")
                || report.peak_memory_source.starts_with("windows:"),
            "peak-memory source describes the captured channel: {}",
            report.peak_memory_source,
        );
    } else {
        assert!(
            report.peak_memory_bytes.is_none(),
            "platforms without a dependency-free source report no peak memory"
        );
        assert_eq!(report.peak_memory_source, "unknown:not-captured");
    }
}
