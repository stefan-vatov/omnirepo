//! Focused proof for plan selection truth tables.

#![allow(dead_code, unused_imports)]

use super::{Policy, SelectionDecision, SelectionError, select_items};
use crate::lifecycle::sync_plan::{PlanDecision, PlanItem};

fn item(id: &str, decision: PlanDecision) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: "t".to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        source_order: 1,
        kind: crate::source::ItemKind::WholeFile,
        decision,
    }
}

fn selected_item(id: &str) -> PlanItem {
    item(
        id,
        PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    )
}

#[test]
fn explicit_include_and_exclude_follow_the_decision_table() {
    let items = vec![
        selected_item("a"),
        selected_item("b"),
        selected_item("c"),
        selected_item("d"),
    ];
    let selections = select_items(
        &items,
        &Policy::Explicit {
            include: vec!["a".to_owned(), "b".to_owned()],
            exclude: vec!["c".to_owned()],
        },
    )
    .expect("select");
    assert!(matches!(
        selections[0].decision,
        SelectionDecision::Selected { .. }
    ));
    assert!(matches!(
        selections[1].decision,
        SelectionDecision::Selected { .. }
    ));
    // c is explicitly excluded; d is outside the explicit scope: both are
    // rejected with their stable reasons.
    let c_reason = match &selections[2].decision {
        SelectionDecision::Rejected { reason } => reason.clone(),
        _ => panic!("expected rejected"),
    };
    assert_eq!(c_reason, "explicit exclude");
    let d_reason = match &selections[3].decision {
        SelectionDecision::Rejected { reason } => reason.clone(),
        _ => panic!("expected rejected"),
    };
    assert_eq!(d_reason, "outside the explicit scope");
}

#[test]
fn unknown_selectors_fail_rather_than_infer() {
    let items = vec![selected_item("a")];
    let error = select_items(
        &items,
        &Policy::Explicit {
            include: vec!["ghost".to_owned()],
            exclude: vec![],
        },
    )
    .expect_err("unknown selector");
    assert!(
        matches!(error, SelectionError::UnknownSelector { .. }),
        "{error}"
    );
    let error = select_items(
        &items,
        &Policy::Explicit {
            include: vec![],
            exclude: vec!["ghost".to_owned()],
        },
    )
    .expect_err("unknown selector");
    assert!(
        matches!(error, SelectionError::UnknownSelector { .. }),
        "{error}"
    );
}

#[test]
fn conflicting_include_and_exclude_fail_typed() {
    let items = vec![selected_item("a")];
    let error = select_items(
        &items,
        &Policy::Explicit {
            include: vec!["a".to_owned()],
            exclude: vec!["a".to_owned()],
        },
    )
    .expect_err("conflict");
    assert!(
        matches!(error, SelectionError::ConflictingSelector { .. }),
        "{error}"
    );
}

#[test]
fn absent_policy_infers_the_canonical_default() {
    let items = vec![
        selected_item("a"),
        item(
            "b",
            PlanDecision::Rejected {
                reason: "shadowed".to_owned(),
            },
        ),
    ];
    let selections = select_items(&items, &Policy::Absent).expect("select");
    assert!(matches!(
        selections[0].decision,
        SelectionDecision::Selected { .. }
    ));
    assert!(matches!(
        selections[1].decision,
        SelectionDecision::Rejected { .. }
    ));
    // Explanations are stable: the absent policy preserves the plan reason.
    assert!(
        format!("{:?}", selections[0].decision).contains("absent policy"),
        "{:?}",
        selections[0].decision
    );
}

#[test]
fn zero_item_plans_are_represented() {
    let selections = select_items(&[], &Policy::Absent).expect("select");
    assert!(selections.is_empty());
    let selections = select_items(
        &[],
        &Policy::Explicit {
            include: vec![],
            exclude: vec![],
        },
    )
    .expect("select");
    assert!(selections.is_empty());
}
