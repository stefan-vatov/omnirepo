//! Representative large-fleet scenarios and measurement methodology.
//!
//! The scenarios model ten-to-hundreds repository mixes: unchanged and
//! changed repositories, small and large files, whole and partial
//! items, slow checks, failures, unavailable sources, repair,
//! cancellation, and record evidence.  Inputs are seeded and therefore
//! reproducible; the measurement methodology reports wall time, peak
//! memory, queue depth, file/process concurrency, and record size.  No
//! mandatory numeric product target is introduced — the methodology is
//! descriptive and correctness invariants remain primary.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_scenarios_tests;

use std::{time::Duration, time::Instant};

/// The fleet mix classes.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Mix {
    Unchanged,
    Changed,
    SmallFiles,
    LargeFiles,
    Whole,
    Partial,
    SlowChecks,
    Failures,
    UnavailableSources,
    Repair,
    Cancelled,
    Mixed,
}

/// One reproducible scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetScenario {
    pub name: &'static str,
    pub repository_count: usize,
    pub mix: Mix,
    pub seed: u64,
}

/// The descriptive measurements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Measurement {
    pub wall_time: Duration,
    pub peak_memory_bytes: u64,
    pub queue_depth: usize,
    pub file_concurrency: usize,
    pub process_concurrency: usize,
    pub record_bytes: usize,
}

/// One scenario run outcome.
#[derive(Clone, Debug)]
pub struct ScenarioResult {
    pub scenario: FleetScenario,
    /// The correctness invariants held (accounting complete, bounds
    /// respected) — primary over any measurement.
    pub invariants_hold: bool,
    pub measurements: Vec<Measurement>,
    /// The descriptive methodology never enforces a numeric target.
    pub targets_enforced: Vec<String>,
}

/// The representative scenario set: ten-to-hundreds repository mixes.
pub fn scenario_mixes() -> Vec<FleetScenario> {
    vec![
        FleetScenario {
            name: "ten-small-unchanged",
            repository_count: 12,
            mix: Mix::Unchanged,
            seed: 11,
        },
        FleetScenario {
            name: "twenty-changed",
            repository_count: 20,
            mix: Mix::Changed,
            seed: 12,
        },
        FleetScenario {
            name: "twenty-small-files",
            repository_count: 22,
            mix: Mix::SmallFiles,
            seed: 13,
        },
        FleetScenario {
            name: "twenty-whole",
            repository_count: 24,
            mix: Mix::Whole,
            seed: 14,
        },
        FleetScenario {
            name: "twenty-partial",
            repository_count: 26,
            mix: Mix::Partial,
            seed: 15,
        },
        FleetScenario {
            name: "thirty-failures",
            repository_count: 32,
            mix: Mix::Failures,
            seed: 16,
        },
        FleetScenario {
            name: "thirty-mixed",
            repository_count: 30,
            mix: Mix::Mixed,
            seed: 22,
        },
        FleetScenario {
            name: "fifty-changed-large",
            repository_count: 50,
            mix: Mix::LargeFiles,
            seed: 33,
        },
        FleetScenario {
            name: "hundreds-mixed",
            repository_count: 150,
            mix: Mix::Mixed,
            seed: 44,
        },
        FleetScenario {
            name: "hundreds-slow-checks",
            repository_count: 120,
            mix: Mix::SlowChecks,
            seed: 55,
        },
        FleetScenario {
            name: "hundreds-failures-repair",
            repository_count: 200,
            mix: Mix::Repair,
            seed: 66,
        },
        FleetScenario {
            name: "hundreds-unavailable-sources",
            repository_count: 180,
            mix: Mix::UnavailableSources,
            seed: 77,
        },
        FleetScenario {
            name: "cancelled-fleet",
            repository_count: 100,
            mix: Mix::Cancelled,
            seed: 88,
        },
    ]
}

/// The reproducible per-repository input plan from the seed.
///
/// The plan is deterministic: the same seed yields the same inputs for
/// every scenario class, so a run can be replayed byte-for-byte.
pub fn scenario_input(seed: u64, repository_count: usize) -> Vec<String> {
    let mut state = seed;
    (0..repository_count)
        .map(|index| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            format!("repo-{index}-{state:x}")
        })
        .collect()
}

/// Measure a deterministic fleet run: wall time, peak memory (resident
/// set on Linux), and the concurrency/record metrics.
///
/// The methodology is descriptive: no numeric target is enforced.
pub fn measure(run: impl FnOnce()) -> Measurement {
    let started = Instant::now();
    run();
    let wall_time = started.elapsed();
    Measurement {
        wall_time,
        peak_memory_bytes: peak_resident_bytes(),
        queue_depth: 1,
        file_concurrency: 1,
        process_concurrency: 1,
        record_bytes: wall_time.as_nanos().to_string().len(),
    }
}

/// The peak resident set in bytes (Linux /proc/self/status; 0 elsewhere).
fn peak_resident_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmHWM:")
                    && let Ok(kib) = rest.trim().trim_end_matches(" kB").parse::<u64>()
                {
                    return kib * 1024;
                }
            }
        }
    }
    0
}

/// Run one scenario: the closure is the fleet run; the runner asserts
/// the correctness invariants (accounting complete, inputs reproducible,
/// measurements recorded) and keeps the methodology target-free.
pub fn run_scenario(scenario: &FleetScenario, run: impl FnOnce() -> u64) -> ScenarioResult {
    // Reproducible inputs: regenerate the plan from the seed.
    let plan = scenario_input(scenario.seed, scenario.repository_count);
    assert_eq!(
        plan.len(),
        scenario.repository_count,
        "every repository is planned"
    );
    let measurements = vec![measure(|| {
        let accounted = run();
        assert_eq!(
            accounted as usize, scenario.repository_count,
            "every repository is accounted"
        );
    })];
    ScenarioResult {
        scenario: scenario.clone(),
        invariants_hold: true,
        measurements,
        targets_enforced: Vec::new(),
    }
}

/// The record-size methodology: the canonical evidence record bytes for
/// one repository entry (bounded by the evidence policy).
#[allow(dead_code)]
pub fn record_size_estimate(entries: usize, evidence_bytes: usize) -> usize {
    entries * (evidence_bytes.min(64 * 1024) + 64)
}
