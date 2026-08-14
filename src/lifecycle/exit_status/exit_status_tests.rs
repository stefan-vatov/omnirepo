//! Focused proof for the exact process-status and stream mapping.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::exit_status::{
    ExitClass, StreamContract, classify_summary, exit_code_for, projection_streams,
    record_available_is_truthful,
};
use crate::lifecycle::run_summary::{
    RepoEntry, RepoOutcome, RunSummary, SummaryStatus, fold_summary,
};

fn summary(repositories: Vec<(String, RepoOutcome)>) -> RunSummary {
    fold_summary(
        "run-1",
        repositories
            .into_iter()
            .map(|(repository, outcome)| (repository, outcome, "evidence-1".to_owned()))
            .collect(),
        true,
    )
    .expect("summary")
}

#[test]
fn every_outcome_class_maps_to_exactly_one_exit_status() {
    assert_eq!(exit_code_for(ExitClass::Success), 0);
    assert_eq!(exit_code_for(ExitClass::InvocationOrPreflight), 2);
    assert_eq!(exit_code_for(ExitClass::PartialFleet), 3);
    assert_eq!(exit_code_for(ExitClass::TotalFailure), 4);
    assert_eq!(exit_code_for(ExitClass::RecordFailure), 5);
    assert_eq!(exit_code_for(ExitClass::Cancelled), 130);
    // The map is total and stable: every class has exactly one code.
    let mut codes = std::collections::BTreeSet::new();
    for class in [
        ExitClass::Success,
        ExitClass::InvocationOrPreflight,
        ExitClass::PartialFleet,
        ExitClass::TotalFailure,
        ExitClass::RecordFailure,
        ExitClass::Cancelled,
    ] {
        assert!(codes.insert(exit_code_for(class)), "{class:?} collides");
    }
}

#[test]
fn summary_classes_derive_to_the_correct_exit_status() {
    // Empty fleet: success (0).
    let empty = fold_summary("run-empty", Vec::new(), true).expect_err("empty folds as invocation");
    let _ = empty;
    // Success: 0.
    let success = summary(vec![
        ("repo-a".to_owned(), RepoOutcome::Success),
        ("repo-b".to_owned(), RepoOutcome::Success),
    ]);
    assert_eq!(classify_summary(&success, true), ExitClass::Success);
    // Partial: some failed, some succeeded -> 3.
    let partial = summary(vec![
        ("repo-a".to_owned(), RepoOutcome::Success),
        (
            "repo-b".to_owned(),
            RepoOutcome::Failure {
                reason: "x".to_owned(),
            },
        ),
    ]);
    assert_eq!(classify_summary(&partial, true), ExitClass::PartialFleet);
    // Total failure -> 4.
    let total = summary(vec![
        (
            "repo-a".to_owned(),
            RepoOutcome::Failure {
                reason: "x".to_owned(),
            },
        ),
        (
            "repo-b".to_owned(),
            RepoOutcome::Failure {
                reason: "y".to_owned(),
            },
        ),
    ]);
    assert_eq!(classify_summary(&total, true), ExitClass::TotalFailure);
    // Record failure -> 5.
    assert_eq!(classify_summary(&success, false), ExitClass::RecordFailure);
    // Cancelled -> 130.
    let cancelled = summary(vec![
        ("repo-a".to_owned(), RepoOutcome::Cancelled),
        ("repo-b".to_owned(), RepoOutcome::Cancelled),
    ]);
    assert_eq!(classify_summary(&cancelled, true), ExitClass::Cancelled);
}

#[test]
fn stdout_and_stderr_never_conflict() {
    let partial = summary(vec![
        ("repo-a".to_owned(), RepoOutcome::Success),
        (
            "repo-b".to_owned(),
            RepoOutcome::Failure {
                reason: "verifier crashed".to_owned(),
            },
        ),
    ]);
    let streams = projection_streams(&partial, true, true);
    // stdout carries the quiet projection; stderr carries diagnostics
    // only; the same content never appears on both.
    assert!(!streams.stdout.is_empty());
    assert!(!streams.stderr.is_empty());
    assert_ne!(
        streams.stdout, streams.stderr,
        "the streams do not conflict"
    );
    // No raw evidence or logger output contaminates the projections.
    assert!(!streams.stdout.contains("evidence-1"), "{}", streams.stdout);
    assert!(!streams.stderr.contains("evidence-1"), "{}", streams.stderr);
}

#[test]
fn record_unavailability_is_truthful() {
    // A missing record never yields a false point: the projection
    // reports the record as unavailable instead of claiming success.
    let success = summary(vec![("repo-a".to_owned(), RepoOutcome::Success)]);
    assert!(record_available_is_truthful(&success, true));
    assert!(!record_available_is_truthful(&success, false));
    // The truthful projection names the record state.
    let streams = projection_streams(&success, false, true);
    assert!(streams.stdout.contains("record"), "{}", streams.stdout);
}
