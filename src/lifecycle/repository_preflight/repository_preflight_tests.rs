//! Focused proof for independent repository preflight.

#![allow(dead_code, unused_imports)]

use super::{RepoPreflight, aggregate_failures, preflight_repositories};
use crate::lifecycle::plan_selection::Policy;
use crate::lifecycle::sync_plan::{PlanDecision, PlanItem};
use crate::source::ItemKind;

fn selected(id: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: "t".to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        source_order: 1,
        kind: ItemKind::WholeFile,
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    }
}

#[test]
fn every_declared_repository_receives_ready_or_failed() {
    let outcomes = preflight_repositories(&[
        ("dest-a", None, &[selected("a")]),
        (
            "dest-b",
            Some(&Policy::Explicit {
                include: vec!["ghost".to_owned()],
                exclude: vec![],
            }),
            &[selected("a")],
        ),
    ]);
    assert_eq!(outcomes.len(), 2);
    assert!(matches!(outcomes[0], RepoPreflight::ReadyPlan { .. }));
    assert!(matches!(outcomes[1], RepoPreflight::Failed { .. }));
}

#[test]
fn true_absence_infers_and_present_invalid_fails_only_that_repository() {
    let outcomes = preflight_repositories(&[
        ("dest-a", None, &[selected("a")]),
        (
            "dest-b",
            Some(&Policy::Explicit {
                include: vec!["ghost".to_owned()],
                exclude: vec![],
            }),
            &[selected("a")],
        ),
        ("dest-c", None, &[selected("c")]),
    ]);
    // dest-a and dest-c infer (ready); dest-b alone fails.
    assert!(matches!(outcomes[0], RepoPreflight::ReadyPlan { .. }));
    assert!(matches!(outcomes[1], RepoPreflight::Failed { .. }));
    assert!(matches!(outcomes[2], RepoPreflight::ReadyPlan { .. }));
}

#[test]
fn all_failures_aggregate_deterministically() {
    let outcomes = preflight_repositories(&[
        (
            "dest-a",
            Some(&Policy::Explicit {
                include: vec!["ghost-a".to_owned()],
                exclude: vec![],
            }),
            &[selected("a")],
        ),
        ("dest-b", None, &[selected("b")]),
        (
            "dest-c",
            Some(&Policy::Explicit {
                include: vec!["ghost-c".to_owned()],
                exclude: vec![],
            }),
            &[selected("c")],
        ),
    ]);
    let failures = aggregate_failures(&outcomes);
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].0, "dest-a", "declared order preserved");
    assert_eq!(failures[1].0, "dest-c");
    assert_eq!(failures[0].1.len(), 1, "one stable reason per failure");
}

#[test]
fn conflicting_policies_fail_only_the_owning_repository() {
    let outcomes = preflight_repositories(&[
        (
            "dest-a",
            Some(&Policy::Explicit {
                include: vec!["a".to_owned()],
                exclude: vec!["a".to_owned()],
            }),
            &[selected("a")],
        ),
        ("dest-b", None, &[selected("b")]),
    ]);
    assert!(matches!(outcomes[0], RepoPreflight::Failed { .. }));
    assert!(matches!(outcomes[1], RepoPreflight::ReadyPlan { .. }));
}
