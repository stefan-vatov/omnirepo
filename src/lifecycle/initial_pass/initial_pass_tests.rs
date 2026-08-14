//! Focused proof for the one-repository initial-pass state machine.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::initial_pass::{InitialResult, InitialStage, InitialState, transition};

#[test]
fn allowed_transitions_are_explicit_and_failures_carry_evidence() {
    let state = InitialState::start("dest-a", "plan-1");
    assert_eq!(state.stage(), InitialStage::PlanFrozen);
    // Plan frozen -> acquired.
    let acquired = state
        .advance(transition::acquired("source-1"))
        .expect("acquire");
    assert_eq!(acquired.stage(), InitialStage::Acquired);
    // Acquired -> synchronized.
    let synchronized = acquired
        .advance(transition::synchronized(InitialResult::Changed))
        .expect("synchronize");
    assert_eq!(synchronized.stage(), InitialStage::Synchronized);
    // A transition with evidence is recorded; a failed result carries the
    // typed reason, and an invalid transition from a terminal stage is a
    // typed error.
    let failed_result = transition::failed("authority-mismatch");
    assert!(format!("{failed_result:?}").contains("authority-mismatch"));
    let cancelled_state = InitialState::start("dest-a", "plan-1")
        .advance(transition::cancelled())
        .expect("cancel");
    let failure = cancelled_state
        .advance(transition::failed("authority-mismatch"))
        .expect_err("invalid transition from the terminal stage");
    assert!(
        format!("{failure}").contains("invalid initial-pass transition"),
        "{failure}"
    );
}

#[test]
fn initial_results_are_distinct_and_not_final_before_repair_folding() {
    let changed = InitialResult::Changed;
    let unchanged = InitialResult::Unchanged;
    let failed = InitialResult::Failed {
        reason: "verify".to_owned(),
    };
    let cancelled = InitialResult::Cancelled;
    let repair_candidate = InitialResult::RepairCandidate {
        reason: "causal repair eligible".to_owned(),
    };
    assert_ne!(changed, unchanged);
    assert_ne!(failed, cancelled);
    assert_ne!(repair_candidate, failed);
    assert!(
        !changed.is_final(),
        "changed is not final before repair folding"
    );
    assert!(
        !repair_candidate.is_final(),
        "repair candidate is not final before repair folding"
    );
    assert!(
        !failed.is_final(),
        "failed is not final before repair folding"
    );
    assert!(cancelled.is_final(), "cancelled is final");
}

#[test]
fn cancelled_and_failed_transitions_are_distinct_stages() {
    let state = InitialState::start("dest-a", "plan-1");
    let cancelled = state.advance(transition::cancelled()).expect("cancel");
    assert_eq!(cancelled.stage(), InitialStage::Cancelled);
    assert!(cancelled.result().as_ref().expect("result").is_cancelled());
    let state = InitialState::start("dest-b", "plan-2");
    let failed = state
        .advance(transition::failed("source-unavailable"))
        .expect("fail");
    assert_eq!(failed.stage(), InitialStage::Failed);
    assert!(failed.result().as_ref().expect("result").is_failed());
}

#[test]
fn handoff_carries_the_repository_and_plan_identity() {
    let state = InitialState::start("dest-a", "plan-1");
    assert_eq!(state.repository(), "dest-a");
    assert_eq!(state.plan_identity(), "plan-1");
    let handoff = state.handoff();
    assert_eq!(handoff.repository, "dest-a");
    assert_eq!(handoff.plan_identity, "plan-1");
}
