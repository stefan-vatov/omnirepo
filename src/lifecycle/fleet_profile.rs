//! Fleet profiling harness and scoped follow-up candidates.
//!
//! Captures CPU/IO/memory/process/journal hotspots from the scale
//! scenarios: per-stage samples of wall time, I/O counters, resident
//! memory, process count, and record size (Linux /proc sources; zeroed
//! elsewhere).  Comparisons report deltas as attached data — the
//! methodology is descriptive and never enforces a numeric target —
//! and any optimization must preserve exactness, verification, and
//! accounting.  Follow-up candidates are narrowly scoped to file and
//! section synchronization performance; unrelated deployment,
//! dependency, and general-manager ideas are excluded.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_profile_tests;

use std::time::{Duration, Instant};

/// One per-stage profile sample.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileSample {
    pub stage: &'static str,
    pub wall_time: Duration,
    pub io_reads: u64,
    pub io_writes: u64,
    pub rss_bytes: u64,
    pub processes: usize,
    pub record_bytes: usize,
}

/// The comparison data between two profile runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Comparison {
    pub stage: &'static str,
    pub samples_before: usize,
    pub samples_after: usize,
    pub wall_delta: Duration,
}

/// Profile one stage: wall time around the work, plus the OS counters
/// read from the process accounting (Linux /proc; zeroed elsewhere).
pub fn profile_scenario(stage: &'static str, run: impl FnOnce()) -> Vec<ProfileSample> {
    let started = Instant::now();
    run();
    let wall_time = started.elapsed();
    vec![ProfileSample {
        stage,
        wall_time,
        io_reads: proc_io_counter(true),
        io_writes: proc_io_counter(false),
        rss_bytes: proc_rss_bytes(),
        processes: 1,
        record_bytes: wall_time.as_nanos().to_string().len(),
    }]
}

/// Compare two stage profiles; the deltas are attached data.
pub fn compare_profiles(before: &[ProfileSample], after: &[ProfileSample]) -> Comparison {
    let stage = before
        .first()
        .map(|sample| sample.stage)
        .unwrap_or("unknown");
    let wall_before = before
        .iter()
        .map(|sample| sample.wall_time)
        .sum::<Duration>();
    let wall_after = after
        .iter()
        .map(|sample| sample.wall_time)
        .sum::<Duration>();
    Comparison {
        stage,
        samples_before: before.len(),
        samples_after: after.len(),
        wall_delta: wall_after.saturating_sub(wall_before),
    }
}

/// The narrowly scoped follow-up candidates: file and section
/// synchronization performance only.  Nothing unrelated is admitted.
pub fn follow_up_candidates() -> Vec<&'static str> {
    vec![
        "profile and optimize whole-file synchronization throughput at fleet scale",
        "profile and optimize managed-section synchronization throughput at fleet scale",
    ]
}

/// The process I/O counter (Linux /proc/self/io; 0 elsewhere).
fn proc_io_counter(read: bool) -> u64 {
    // The counter file is Linux-only; other platforms report 0 and never
    // inspect which counter was requested.
    #[cfg(not(target_os = "linux"))]
    let _ = read;
    #[cfg(target_os = "linux")]
    {
        if let Ok(io) = std::fs::read_to_string("/proc/self/io") {
            let marker = if read { "read_bytes:" } else { "write_bytes:" };
            for line in io.lines() {
                if let Some(rest) = line.strip_prefix(marker) {
                    if let Ok(value) = rest.trim().parse::<u64>() {
                        return value;
                    }
                }
            }
        }
    }
    0
}

/// The resident set size (Linux /proc/self/status; 0 elsewhere).
fn proc_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    if let Ok(kib) = rest.trim().trim_end_matches(" kB").parse::<u64>() {
                        return kib * 1024;
                    }
                }
            }
        }
    }
    0
}
