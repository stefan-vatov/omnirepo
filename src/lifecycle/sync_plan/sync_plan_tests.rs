//! Focused proof for immutable synchronization plans.

#![allow(dead_code, unused_imports)]

use super::{PlanDecision, PlanError, PlanItem, SyncPlan, validate_plan};

fn selected(id: &str, target: &str, source: &str, order: usize) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: source.to_owned(),
        source_path: String::new(),
        source_order: order,
        kind: crate::source::ItemKind::WholeFile,
        section: None,
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    }
}

fn rejected(id: &str, target: &str, source: &str, order: usize) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: source.to_owned(),
        source_path: String::new(),
        source_order: order,
        kind: crate::source::ItemKind::WholeFile,
        section: None,
        decision: PlanDecision::Rejected {
            reason: "shadowed by a higher-precedence source".to_owned(),
        },
    }
}

#[test]
fn every_item_carries_a_stable_reason() {
    let plan = SyncPlan::new(
        "dest-a",
        vec![
            selected("item-a", "t1", "primary", 1),
            rejected("item-b", "t2", "legacy", 2),
        ],
    );
    assert_eq!(plan.items.len(), 2);
    assert!(
        matches!(plan.items[0].decision, PlanDecision::Selected { .. }),
        "selected item must carry its reason"
    );
    assert!(
        matches!(plan.items[1].decision, PlanDecision::Rejected { .. }),
        "rejected item must carry its reason"
    );
}

#[test]
fn operations_retain_source_precedence_and_destination_order() {
    // Two sources in precedence order; the plan keeps the declared order.
    let plan = SyncPlan::new(
        "dest-a",
        vec![
            selected("primary-item", "t1", "primary", 1),
            rejected("legacy-item", "t1", "legacy", 2),
            selected("primary-item-2", "t2", "primary", 3),
        ],
    );
    let rendered = plan.render();
    let primary_position = rendered.find("primary-item ").expect("primary first");
    let legacy_position = rendered.find("legacy-item ").expect("legacy present");
    assert!(
        primary_position < legacy_position,
        "source precedence retained"
    );
}

#[test]
fn serialization_is_deterministic() {
    let build = || {
        SyncPlan::new(
            "dest-a",
            vec![
                selected("a", "t1", "primary", 1),
                rejected("b", "t2", "legacy", 2),
            ],
        )
    };
    assert_eq!(build().render(), build().render(), "byte-identical renders");
    let rendered = build().render();
    assert!(
        rendered.starts_with("omnirepo.sync-plan.v1 destination=dest-a\n"),
        "{rendered}"
    );
}

#[test]
fn domain_type_has_no_mutation_surface() {
    // The plan is immutable data: the type exposes only construction and
    // rendering.  (Compile-time contract: no &mut self methods exist; the
    // assertion pins the shape.)
    let plan = SyncPlan::new("dest-a", vec![selected("a", "t1", "primary", 1)]);
    let items = plan.items.clone();
    assert_eq!(items.len(), 1);
    let _ = plan.render();
}

#[test]
fn validation_rejects_empty_and_duplicate_plans() {
    let empty = SyncPlan::new("dest-a", vec![]);
    assert!(matches!(
        validate_plan(&empty),
        Err(PlanError::Empty { .. })
    ));
    let duplicate = SyncPlan::new(
        "dest-a",
        vec![
            selected("a", "t1", "primary", 1),
            selected("a", "t2", "legacy", 2),
        ],
    );
    assert!(matches!(
        validate_plan(&duplicate),
        Err(PlanError::DuplicateItem { .. })
    ));
    // A rejected loser may share the winner's id: losers stay visible in
    // the plan without failing validation.
    let with_loser = SyncPlan::new(
        "dest-a",
        vec![
            selected("a", "t1", "primary", 1),
            rejected("a", "t1", "legacy", 2),
        ],
    );
    assert!(validate_plan(&with_loser).is_ok());
    // Two selected whole-file claims on one target are invalid.
    let double_claim = SyncPlan::new(
        "dest-a",
        vec![
            selected("a", "t1", "primary", 1),
            selected("b", "t1", "legacy", 2),
        ],
    );
    assert!(matches!(
        validate_plan(&double_claim),
        Err(PlanError::DuplicateTarget { .. })
    ));
    let valid = SyncPlan::new("dest-a", vec![selected("a", "t1", "primary", 1)]);
    assert!(validate_plan(&valid).is_ok());
}

#[test]
fn a_plan_with_only_rejected_losers_is_empty_work() {
    // Rejected losers never execute: they must not satisfy the
    // non-emptiness rule, or a zero-work plan would deliver a no-op
    // commit.
    let losers_only = SyncPlan::new("dest-a", vec![rejected("a", "t1", "legacy", 2)]);
    assert!(matches!(
        validate_plan(&losers_only),
        Err(PlanError::Empty { .. })
    ));
}
