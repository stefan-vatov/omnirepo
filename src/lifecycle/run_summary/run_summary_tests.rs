//! Focused proof for durable run summary folding.

#![allow(dead_code, unused_imports)]

use super::{RepoOutcome, RunSummary, SummaryError, SummaryStatus, fold_summary, render_summary};

fn success(repo: &str) -> (String, RepoOutcome, String) {
    (
        repo.to_owned(),
        RepoOutcome::Success,
        format!("evidence/{repo}/result.json"),
    )
}

fn failure(repo: &str) -> (String, RepoOutcome, String) {
    (
        repo.to_owned(),
        RepoOutcome::Failure {
            reason: format!("{repo} verification failed"),
        },
        format!("evidence/{repo}/result.json"),
    )
}

fn cancelled(repo: &str) -> (String, RepoOutcome, String) {
    (
        repo.to_owned(),
        RepoOutcome::Cancelled,
        format!("evidence/{repo}/result.json"),
    )
}

#[test]
fn every_repository_has_exactly_one_terminal_outcome() {
    let summary = fold_summary(
        "run-1",
        vec![success("dest-a"), failure("dest-b"), cancelled("dest-c")],
        true,
    )
    .expect("fold");
    assert_eq!(summary.repositories.len(), 3);
    assert_eq!(
        summary.repositories[1].outcome,
        RepoOutcome::Failure {
            reason: "dest-b verification failed".to_owned()
        }
    );
    // A duplicated outcome fails typed: no failure is hidden or doubled.
    let error =
        fold_summary("run-1", vec![success("a"), success("a")], true).expect_err("duplicate");
    assert!(
        matches!(error, SummaryError::DuplicateOutcome { .. }),
        "{error}"
    );
    // An empty run fails typed.
    assert!(matches!(
        fold_summary("run-1", vec![], true),
        Err(SummaryError::Empty)
    ));
}

#[test]
fn status_derives_from_outcomes_and_record_state() {
    let all_ok = fold_summary("run-1", vec![success("a"), success("b")], true).expect("fold");
    assert_eq!(all_ok.status, SummaryStatus::Success);
    let failed = fold_summary("run-1", vec![success("a"), failure("b")], true).expect("fold");
    assert_eq!(failed.status, SummaryStatus::Failed);
    let cancelled_run = fold_summary("run-1", vec![cancelled("a")], true).expect("fold");
    assert_eq!(cancelled_run.status, SummaryStatus::Cancelled);
    // A non-terminal record yields an incomplete summary even when all
    // listed outcomes succeeded.
    let incomplete = fold_summary("run-1", vec![success("a")], false).expect("fold");
    assert_eq!(incomplete.status, SummaryStatus::Incomplete);
}

#[test]
fn serialization_is_deterministic_and_bounded() {
    let build = || fold_summary("run-1", vec![success("a"), failure("b")], true).expect("fold");
    assert_eq!(render_summary(&build()), render_summary(&build()));
    let rendered = render_summary(&build());
    assert!(
        rendered.starts_with("omnirepo.run-summary.v1 run=run-1 "),
        "{rendered}"
    );
    assert!(rendered.contains("repo=a outcome=success"), "{rendered}");
    assert!(rendered.contains("repo=b outcome=failure"), "{rendered}");
}

#[test]
fn evidence_references_are_bounded_paths_only() {
    let summary = fold_summary("run-1", vec![success("a")], true).expect("fold");
    // The evidence field is a bounded reference, never raw output.
    assert_eq!(summary.repositories[0].evidence, "evidence/a/result.json");
    let _: &RunSummary = &summary;
}
