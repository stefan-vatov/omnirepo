//! Focused proof for terminal projections.

#![allow(dead_code, unused_imports)]

use super::{PROJECTION_SCHEMA, render_human, render_machine};
use crate::lifecycle::run_summary::{RepoOutcome, RunSummary, SummaryStatus, fold_summary};

fn summary(outcomes: Vec<(String, RepoOutcome, String)>, terminal: bool) -> RunSummary {
    fold_summary("run-1", outcomes, terminal).expect("fold")
}

#[test]
fn success_emits_the_decided_zero_or_one_line() {
    let ok = summary(
        vec![(
            "a".to_owned(),
            RepoOutcome::Success,
            "evidence/a".to_owned(),
        )],
        true,
    );
    // Decided zero lines.
    assert_eq!(render_human(&ok, true, None), "");
    // Decided one concise line.
    assert_eq!(render_human(&ok, true, Some("sync: ok")), "sync: ok\n");
}

#[test]
fn failure_names_every_affected_repository_and_the_record() {
    let failed = summary(
        vec![
            (
                "a".to_owned(),
                RepoOutcome::Success,
                "evidence/a".to_owned(),
            ),
            (
                "b".to_owned(),
                RepoOutcome::Failure {
                    reason: "verify".to_owned(),
                },
                "evidence/b".to_owned(),
            ),
            (
                "c".to_owned(),
                RepoOutcome::Failure {
                    reason: "push".to_owned(),
                },
                "evidence/c".to_owned(),
            ),
        ],
        true,
    );
    let human = render_human(&failed, true, None);
    assert!(human.contains("b, c"), "{human}");
    assert!(human.contains("record run-1"), "{human}");
    assert!(
        !human.contains("failed: a"),
        "successful peers are not named as failures: {human}"
    );
}

#[test]
fn record_unavailable_has_a_truthful_alternative() {
    let failed = summary(
        vec![(
            "b".to_owned(),
            RepoOutcome::Failure {
                reason: "verify".to_owned(),
            },
            "evidence/b".to_owned(),
        )],
        true,
    );
    let human = render_human(&failed, false, None);
    assert!(human.contains("record is not available"), "{human}");
    assert!(!human.contains("record run-1"), "{human}");
    let cancelled = summary(
        vec![(
            "b".to_owned(),
            RepoOutcome::Cancelled,
            "evidence/b".to_owned(),
        )],
        true,
    );
    let human = render_human(&cancelled, false, None);
    assert!(human.contains("record is not available"), "{human}");
}

#[test]
fn machine_mode_has_no_human_contamination() {
    let failed = summary(
        vec![
            (
                "a".to_owned(),
                RepoOutcome::Success,
                "evidence/a/result.json".to_owned(),
            ),
            (
                "b".to_owned(),
                RepoOutcome::Failure {
                    reason: "verify".to_owned(),
                },
                "evidence/b/result.json".to_owned(),
            ),
        ],
        true,
    );
    let machine = render_machine(&failed, true);
    for line in machine.lines() {
        assert!(line.starts_with('{'), "machine lines are JSON: {line}");
        assert!(line.ends_with('}'), "machine lines are JSON: {line}");
    }
    assert!(machine.contains(PROJECTION_SCHEMA), "{machine}");
    assert!(machine.contains("\"status\":\"failed\""), "{machine}");
    assert!(!machine.contains("sync failed"), "no human text: {machine}");
    assert!(!machine.contains("see record"), "no human text: {machine}");
}
