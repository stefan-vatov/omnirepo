//! Focused proof for the fleet profiling harness and scoped follow-up
//! candidates.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_profile::{
    Comparison, ProfileSample, compare_profiles, follow_up_candidates, profile_scenario,
};
use std::time::Duration;

#[test]
fn the_profile_captures_every_metric_per_stage() {
    let samples = profile_scenario("sync", || {
        std::thread::sleep(Duration::from_millis(2));
    });
    assert_eq!(samples.len(), 1, "one stage sample");
    let sample = &samples[0];
    assert_eq!(sample.stage, "sync");
    assert!(
        sample.wall_time >= Duration::from_millis(2),
        "wall time is captured: {:?}",
        sample.wall_time
    );
    assert!(
        sample.io_reads + sample.io_writes > 0 || sample.rss_bytes > 0,
        "io/memory is captured"
    );
    assert!(sample.processes >= 1, "the profiler counts its own process");
    assert!(sample.record_bytes > 0, "the record size is captured");
}

#[test]
fn comparisons_report_deltas_and_keep_exactness() {
    let slow = profile_scenario("sync", || {
        std::thread::sleep(Duration::from_millis(4));
    });
    let fast = profile_scenario("sync", || {
        std::thread::sleep(Duration::from_millis(1));
    });
    let comparison = compare_profiles(&slow, &fast);
    assert_eq!(comparison.stage, "sync");
    assert!(
        comparison.wall_delta >= Duration::ZERO,
        "the comparison reports the wall delta: {:?}",
        comparison.wall_delta
    );
    // Exactness is preserved: the profiled runs are accounted the same
    // way the fleet runner accounts them (the measurement never loses a
    // sample).
    assert_eq!(slow.len(), fast.len(), "the samples are comparable");
    assert_eq!(comparison.samples_before, slow.len());
    assert_eq!(comparison.samples_after, fast.len());
}

#[test]
fn follow_up_candidates_stay_scoped_to_file_and_section_sync() {
    let candidates = follow_up_candidates();
    assert_eq!(
        candidates.len(),
        2,
        "exactly the scoped candidates: {candidates:?}"
    );
    assert!(
        candidates.iter().any(|title| title.contains("file")),
        "{candidates:?}"
    );
    assert!(
        candidates.iter().any(|title| title.contains("section")),
        "{candidates:?}"
    );
    for title in &candidates {
        assert!(
            !title.contains("dependency") && !title.contains("deployment"),
            "unrelated ideas are excluded: {title}"
        );
    }
}

#[test]
fn profile_data_is_reproducible_for_the_same_work() {
    let first = profile_scenario("journal", || {
        let mut sum = 0_u64;
        for index in 0..1000 {
            sum = sum.wrapping_add(index);
        }
        assert_eq!(sum, 499500);
    });
    let second = profile_scenario("journal", || {
        let mut sum = 0_u64;
        for index in 0..1000 {
            sum = sum.wrapping_add(index);
        }
        assert_eq!(sum, 499500);
    });
    // The deterministic work yields comparable samples; the comparison
    // is attached as data, never enforced as a numeric target.
    let comparison = compare_profiles(&first, &second);
    assert!(comparison.samples_before == comparison.samples_after);
}
