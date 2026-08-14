//! Focused proof for folding the initial and repair histories into one
//! terminal repository outcome.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_fold::{RepairHistory, fold_into_terminal_outcome};
use crate::lifecycle::run_summary::RepoOutcome;

fn failed(reason: &str) -> RepoOutcome {
    RepoOutcome::Failure {
        reason: reason.to_owned(),
    }
}

#[test]
fn a_successful_repair_upgrades_the_failure_to_a_terminal_success() {
    let initial = failed("sync drift");
    let repair = RepairHistory::Succeeded {
        oid: "abc123".to_owned(),
    };
    let outcome = fold_into_terminal_outcome(&initial, Some(&repair));
    assert!(matches!(outcome, RepoOutcome::Success), "{outcome:?}");
}

#[test]
fn a_failed_repair_keeps_the_initial_failure_with_the_repair_reason() {
    let initial = failed("sync drift");
    let repair = RepairHistory::Failed {
        reason: "agent crashed".to_owned(),
    };
    let outcome = fold_into_terminal_outcome(&initial, Some(&repair));
    match outcome {
        RepoOutcome::Failure { reason } => {
            assert!(reason.contains("sync drift"), "{reason}");
            assert!(reason.contains("agent crashed"), "{reason}");
        }
        other => panic!("expected failure, got {other:?}"),
    }
}

#[test]
fn an_untouched_repository_keeps_its_initial_outcome() {
    let initial = failed("verification failed");
    let outcome = fold_into_terminal_outcome(&initial, None);
    assert_eq!(outcome, initial);
}

#[test]
fn an_initial_success_stays_success_even_after_a_repair_attempt() {
    let initial = RepoOutcome::Success;
    let repair = RepairHistory::Failed {
        reason: "agent crashed".to_owned(),
    };
    let outcome = fold_into_terminal_outcome(&initial, Some(&repair));
    assert!(matches!(outcome, RepoOutcome::Success), "{outcome:?}");
}

#[test]
fn a_cancelled_repository_stays_cancelled() {
    let initial = RepoOutcome::Cancelled;
    let repair = RepairHistory::Succeeded {
        oid: "abc123".to_owned(),
    };
    let outcome = fold_into_terminal_outcome(&initial, Some(&repair));
    assert_eq!(outcome, RepoOutcome::Cancelled);
}
