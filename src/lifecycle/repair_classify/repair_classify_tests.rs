//! Focused proof for exhaustive failure-stage to repair-eligibility
//! classification.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_classify::{
    Eligibility, FailureClass, RepairClassification, classify_failure,
};

#[test]
fn only_selected_sync_and_verification_classes_can_proceed() {
    // Sync-induced and verification failures are eligible for bounded
    // repair.
    let sync_drift = classify_failure(FailureClass::SyncDrift);
    assert!(matches!(
        sync_drift.eligibility,
        Eligibility::Repairable { .. }
    ));
    let verification = classify_failure(FailureClass::VerificationFailed);
    assert!(matches!(
        verification.eligibility,
        Eligibility::Repairable { .. }
    ));
    // A repair attempt failure stays repairable only within the attempt
    // budget; the classifier marks the class.
    let repair_attempt = classify_failure(FailureClass::RepairAttemptFailed);
    assert!(matches!(
        repair_attempt.eligibility,
        Eligibility::Repairable { .. }
    ));
}

#[test]
fn shared_authority_git_journal_and_unrelated_classes_are_terminal() {
    for class in [
        FailureClass::MachineAuthorityInvalid,
        FailureClass::SourceAuthorityInvalid,
        FailureClass::GitDeliveryFailed,
        FailureClass::JournalFailure,
        FailureClass::Unrelated,
        FailureClass::Uncertain,
    ] {
        let classification = classify_failure(class);
        assert_eq!(
            classification.eligibility,
            Eligibility::Terminal,
            "{class:?} must be terminal"
        );
    }
}

#[test]
fn classification_is_pure_and_explainable() {
    // The same class always yields the same typed classification with a
    // stable explanation.
    let first = classify_failure(FailureClass::SyncDrift);
    let second = classify_failure(FailureClass::SyncDrift);
    assert_eq!(first, second);
    assert!(!first.explanation.is_empty());
    let _: RepairClassification = first;
}

#[test]
fn classification_is_exhaustive() {
    // Every failure class maps to a classification: the classifier is
    // total over the declared classes.
    let classes = [
        FailureClass::SyncDrift,
        FailureClass::VerificationFailed,
        FailureClass::RepairAttemptFailed,
        FailureClass::MachineAuthorityInvalid,
        FailureClass::SourceAuthorityInvalid,
        FailureClass::GitDeliveryFailed,
        FailureClass::JournalFailure,
        FailureClass::Unrelated,
        FailureClass::Uncertain,
    ];
    for class in classes {
        let _ = classify_failure(class);
    }
}
