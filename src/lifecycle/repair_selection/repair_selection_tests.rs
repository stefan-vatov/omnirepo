//! Focused proof for selecting eligible failed repositories after the
//! complete initial pass.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_causation::CausationVerdict;
use crate::lifecycle::repair_classify::FailureClass;
use crate::lifecycle::repair_selection::{FailedRepository, select_eligible_failed};

fn failed(repository: &str, class: FailureClass) -> FailedRepository {
    FailedRepository {
        repository: repository.to_owned(),
        class,
    }
}

fn proven(repository: &str) -> (String, CausationVerdict) {
    (repository.to_owned(), CausationVerdict::Proven)
}

fn not_proven(repository: &str) -> (String, CausationVerdict) {
    (
        repository.to_owned(),
        CausationVerdict::NotProven {
            reason: "baseline mismatch".to_owned(),
        },
    )
}

#[test]
fn repairable_and_proven_repositories_are_selected_in_input_order() {
    let failed = vec![
        failed("repo-a", FailureClass::SyncDrift),
        failed("repo-b", FailureClass::VerificationFailed),
        failed("repo-c", FailureClass::RepairAttemptFailed),
    ];
    let causation = vec![proven("repo-a"), proven("repo-b"), proven("repo-c")];
    let selected = select_eligible_failed(&failed, &causation);
    let ids = selected
        .iter()
        .map(|entry| entry.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-a", "repo-b", "repo-c"], "{ids:?}");
    for entry in &selected {
        assert!(entry.attempts >= 1, "{entry:?}");
        assert!(!entry.reason.is_empty(), "{entry:?}");
    }
}

#[test]
fn terminal_classes_are_never_selected() {
    let failed = vec![
        failed("repo-a", FailureClass::Uncertain),
        failed("repo-b", FailureClass::Unrelated),
        failed("repo-c", FailureClass::JournalFailure),
        failed("repo-d", FailureClass::MachineAuthorityInvalid),
        failed("repo-e", FailureClass::SourceAuthorityInvalid),
        failed("repo-f", FailureClass::GitDeliveryFailed),
    ];
    let causation = vec![
        proven("repo-a"),
        proven("repo-b"),
        proven("repo-c"),
        proven("repo-d"),
        proven("repo-e"),
        proven("repo-f"),
    ];
    assert!(select_eligible_failed(&failed, &causation).is_empty());
}

#[test]
fn unproven_causation_excludes_an_otherwise_repairable_repository() {
    let failed = vec![
        failed("repo-a", FailureClass::SyncDrift),
        failed("repo-b", FailureClass::SyncDrift),
    ];
    let causation = vec![proven("repo-a"), not_proven("repo-b")];
    let selected = select_eligible_failed(&failed, &causation);
    assert_eq!(selected.len(), 1, "{selected:?}");
    assert_eq!(selected[0].repository, "repo-a");
}

#[test]
fn an_empty_failed_set_selects_nothing() {
    assert!(select_eligible_failed(&[], &[]).is_empty());
}
