//! Focused proof for proving current-run causation from baseline and
//! frozen lineage.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_causation::{CausationVerdict, prove_current_run_causation};

#[test]
fn matching_baseline_and_recorded_effect_prove_causation() {
    let verdict = prove_current_run_causation("lineage-1", "lineage-1", true);
    assert_eq!(verdict, CausationVerdict::Proven);
}

#[test]
fn lineage_mismatch_fails_causation() {
    let verdict = prove_current_run_causation("baseline-1", "lineage-2", true);
    assert!(matches!(verdict, CausationVerdict::NotProven { .. }));
}

#[test]
fn missing_effect_fails_causation() {
    let verdict = prove_current_run_causation("baseline-1", "lineage-1", false);
    assert!(matches!(verdict, CausationVerdict::NotProven { .. }));
}

#[test]
fn empty_baseline_fails_causation() {
    let verdict = prove_current_run_causation("", "lineage-1", true);
    assert!(matches!(verdict, CausationVerdict::NotProven { .. }));
}
