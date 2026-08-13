//! Summary completeness, persistence-failure, and output snapshot
//! fixtures.

#![allow(dead_code, unused_imports)]

use super::{render_human, render_machine, sanitize_id};
use crate::lifecycle::run_summary::{
    RepoOutcome, RunSummary, SummaryStatus, fold_summary, render_summary,
};

fn summary(outcomes: Vec<(String, RepoOutcome, String)>, terminal: bool) -> RunSummary {
    fold_summary("run-1", outcomes, terminal).expect("fold")
}

#[test]
fn durable_summary_contains_complete_outcomes() {
    let s = summary(
        vec![
            (
                "a".to_owned(),
                RepoOutcome::Success,
                "evidence/a".to_owned(),
            ),
            (
                "b".to_owned(),
                RepoOutcome::Cancelled,
                "evidence/b".to_owned(),
            ),
        ],
        true,
    );
    let rendered = render_summary(&s);
    assert!(rendered.contains("repo=a outcome=success"), "{rendered}");
    assert!(rendered.contains("repo=b outcome=cancelled"), "{rendered}");
    // Every repository is accounted exactly once.
    assert_eq!(rendered.matches("repo=").count(), 2, "{rendered}");
}

#[test]
fn terminal_projection_matches_the_contract_exactly() {
    let ok = summary(
        vec![(
            "a".to_owned(),
            RepoOutcome::Success,
            "evidence/a".to_owned(),
        )],
        true,
    );
    assert_eq!(
        render_human(&ok, true, None),
        "",
        "success: zero lines when decided"
    );
    assert_eq!(
        render_human(&ok, true, Some("sync: ok")),
        "sync: ok\n",
        "one concise line"
    );
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
    let human = render_human(&failed, true, None);
    assert_eq!(
        human, "sync failed: b; see record run-1\n",
        "exact failure line"
    );
    let machine = render_machine(&failed, true);
    assert!(
        machine.starts_with("{\"schema\":\"omnirepo.terminal-projection.v1\""),
        "{machine}"
    );
}

#[test]
fn no_ansi_osc_or_newline_injection_and_no_interleaving() {
    let hostile = "evil\u{1b}[2Jrepo\nsecond-line";
    let sanitized = sanitize_id(hostile);
    assert!(!sanitized.contains('\u{1b}'), "{sanitized:?}");
    assert!(!sanitized.contains('\n'), "{sanitized:?}");
    assert!(
        !sanitized.chars().any(|c| (c as u32) < 0x20),
        "no control characters remain: {sanitized:?}"
    );
    let s = summary(
        vec![(
            "evil\u{1b}[2Jrepo".to_owned(),
            RepoOutcome::Failure {
                reason: "x".to_owned(),
            },
            "evidence\u{1b}]0;title".to_owned(),
        )],
        true,
    );
    let human = render_human(&s, true, None);
    assert!(!human.contains('\u{1b}'), "{human:?}");
    let machine = render_machine(&s, true);
    for line in machine.lines() {
        assert!(!line.contains('\u{1b}'), "{line:?}");
        assert!(line.starts_with('{') && line.ends_with('}'), "{line:?}");
    }
}

#[test]
fn no_false_success_or_false_pointer() {
    // A record-unavailable failure never claims a record reference.
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
    assert!(!human.contains("see record run-1"), "{human}");
    // A success summary never names failures.
    let ok = summary(
        vec![(
            "a".to_owned(),
            RepoOutcome::Success,
            "evidence/a".to_owned(),
        )],
        true,
    );
    assert!(!render_human(&ok, true, None).contains("failed"), "{human}");
}

#[test]
fn output_order_is_stable_where_promised() {
    // The machine projection follows the folded repository order.
    let s = summary(
        vec![
            (
                "z".to_owned(),
                RepoOutcome::Success,
                "evidence/z".to_owned(),
            ),
            (
                "a".to_owned(),
                RepoOutcome::Cancelled,
                "evidence/a".to_owned(),
            ),
        ],
        true,
    );
    let machine = render_machine(&s, true);
    let z = machine.find("\"repo\":\"z\"").expect("z present");
    let a = machine.find("\"repo\":\"a\"").expect("a present");
    assert!(z < a, "folded order is preserved: {machine}");
    assert_eq!(machine.lines().count(), 3, "header plus two entries");
}
