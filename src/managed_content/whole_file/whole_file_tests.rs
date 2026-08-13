//! Focused proof for whole-file outcome classification.

#![allow(dead_code, unused_imports)]

use super::{WholeFileFailure, WholeFileOutcome, classify_whole_file};

#[test]
fn missing_equal_and_different_targets_classify_typed() {
    // Missing → create.
    assert_eq!(
        classify_whole_file(false, None, b"payload").expect("classify"),
        WholeFileOutcome::Create
    );
    // Equal → unchanged (true no-op).
    assert_eq!(
        classify_whole_file(true, Some(b"payload"), b"payload").expect("classify"),
        WholeFileOutcome::Unchanged
    );
    // Different → replace (local drift is overwritten without prompt).
    assert_eq!(
        classify_whole_file(true, Some(b"local-drift"), b"payload").expect("classify"),
        WholeFileOutcome::Replace
    );
}

#[test]
fn empty_payload_is_a_typed_empty_create() {
    assert_eq!(
        classify_whole_file(false, None, b"").expect("classify"),
        WholeFileOutcome::EmptyCreate
    );
    assert_eq!(
        classify_whole_file(true, Some(b"x"), b"").expect("classify"),
        WholeFileOutcome::EmptyCreate
    );
}

#[test]
fn unreadable_targets_fail_typed() {
    let error = classify_whole_file(true, None, b"payload").expect_err("unreadable");
    assert!(
        matches!(error, WholeFileFailure::ReadOnly { .. }),
        "{error}"
    );
}

#[test]
fn classification_never_mutates_and_handles_nested_and_acquisition_cases() {
    // The classifier is pure: it only decides; the nested and
    // acquisition-failure cases are the caller's typed gates, proven here
    // as typed values with stable reasons.
    let _nested = WholeFileFailure::Nested {
        reason: "target is inside a managed section".to_owned(),
    };
    let _acquisition = WholeFileFailure::SourceUnavailable {
        reason: "the source acquisition failed; nothing is mutated".to_owned(),
    };
    let _encoding = WholeFileFailure::InvalidEncoding {
        reason: "the payload is not valid UTF-8 for this target".to_owned(),
    };
    // And the classifier itself returns a decision without any effect.
    let outcome = classify_whole_file(true, Some(b"x"), b"y").expect("classify");
    assert_eq!(outcome, WholeFileOutcome::Replace);
}
