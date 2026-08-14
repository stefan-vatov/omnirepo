//! Focused proof for the representative large-fleet scenarios and
//! measurement methodology.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_scenarios::{
    FleetScenario, Measurement, Mix, measure, run_scenario, scenario_mixes,
};

#[test]
fn the_scenario_mix_covers_ten_to_hundreds_of_repositories() {
    let scenarios = scenario_mixes();
    assert!(!scenarios.is_empty());
    let counts = scenarios
        .iter()
        .map(|scenario| scenario.repository_count)
        .collect::<Vec<_>>();
    assert!(
        counts.iter().any(|count| *count >= 10 && *count <= 50),
        "the ten-to-fifty mix is present: {counts:?}"
    );
    assert!(
        counts.iter().any(|count| *count > 100),
        "the hundreds mix is present: {counts:?}"
    );
}

#[test]
fn every_required_fleet_class_is_modeled() {
    let scenarios = scenario_mixes();
    let mixes = scenarios
        .iter()
        .map(|scenario| scenario.mix)
        .collect::<std::collections::BTreeSet<_>>();
    for required in [
        Mix::Unchanged,
        Mix::Changed,
        Mix::SmallFiles,
        Mix::LargeFiles,
        Mix::Whole,
        Mix::Partial,
        Mix::SlowChecks,
        Mix::Failures,
        Mix::UnavailableSources,
        Mix::Repair,
        Mix::Cancelled,
        Mix::Mixed,
    ] {
        assert!(
            mixes.contains(&required),
            "missing fleet class {required:?}"
        );
    }
}

#[test]
fn scenario_inputs_are_reproducible_from_the_seed() {
    // The same seed produces the same scenario input plan.
    let first = scenario_input(42);
    let second = scenario_input(42);
    assert_eq!(first, second, "seeded inputs are reproducible");
    let other = scenario_input(7);
    assert_ne!(first, other, "a different seed yields a different plan");
}

fn scenario_input(seed: u64) -> Vec<String> {
    crate::lifecycle::fleet_scenarios::scenario_input(seed, 8)
}

#[test]
fn the_measurement_methodology_reports_all_metrics() {
    let measurement = measure(|| {
        // A deterministic synthetic fleet: 20 repos, each one accounted.
        let mut accounted = 0;
        for _ in 0..20 {
            accounted += 1;
        }
        assert_eq!(accounted, 20, "every repository is accounted");
    });
    assert!(measurement.queue_depth >= 1, "the runner queue is measured");
    assert!(measurement.file_concurrency >= 1, "files are processed");
    assert!(measurement.process_concurrency >= 1, "at least the runner");
    assert!(measurement.record_bytes > 0, "the run record has bytes");
    #[cfg(target_os = "linux")]
    assert!(measurement.peak_memory_bytes > 0, "peak memory is reported");
}

#[test]
fn the_scenario_runner_keeps_correctness_invariants_primary() {
    let scenarios = scenario_mixes();
    for scenario in &scenarios {
        let count = scenario.repository_count as u64;
        let result = run_scenario(scenario, move || count);
        assert!(
            result.invariants_hold,
            "scenario {} violates the correctness invariants",
            scenario.name
        );
        assert!(
            !result.measurements.is_empty(),
            "scenario {} produced measurements",
            scenario.name
        );
        // The methodology is descriptive: no mandatory numeric product
        // target is enforced by the runner itself.
        assert!(result.targets_enforced.is_empty());
    }
}
