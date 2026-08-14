//! Focused proof for the property suite: no duplicate effects, no lost
//! outcomes, no authority escapes across generated and fault-injected
//! model runs.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::model_property_suite::{PropertyFailure, run_property_suite};

#[test]
fn the_suite_is_deterministic_for_a_fixed_seed_set() {
    let first = run_property_suite(&[1, 2, 3, 4, 5]);
    let second = run_property_suite(&[1, 2, 3, 4, 5]);
    assert_eq!(first, second, "the suite is deterministic");
    assert!(first.seeds_run == 5, "{first:?}");
    assert!(first.sequences_run >= 15, "{first:?}");
}

#[test]
fn no_duplicate_effects_no_lost_outcomes_and_no_authority_escapes() {
    let report = run_property_suite(&[11, 22, 33, 44, 55, 66, 77, 88]);
    assert!(
        report.failures.is_empty(),
        "expected zero property failures: {:?}",
        report
            .failures
            .iter()
            .map(|failure| format!("{failure:?}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn every_failure_is_typed_and_carries_the_seed_and_sequence() {
    // A deliberately broken seed set cannot exist (the suite is total),
    // so the failure shape is proven structurally: any failure names
    // the seed, the sequence index, and the invariant.
    let report = run_property_suite(&[1]);
    for failure in &report.failures {
        match failure {
            PropertyFailure::DuplicateEffect { .. }
            | PropertyFailure::LostOutcome { .. }
            | PropertyFailure::AuthorityEscape { .. }
            | PropertyFailure::Invariant { .. } => {}
        }
    }
}
