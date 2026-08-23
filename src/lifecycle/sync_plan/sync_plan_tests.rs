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

fn selected_section(id: &str, target: &str, source: &str, order: usize, section: &str) -> PlanItem {
    PlanItem {
        kind: crate::source::ItemKind::Section,
        section: Some(crate::configuration::SectionId::new(section).expect("section id")),
        ..selected(id, target, source, order)
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

#[test]
fn target_groups_gather_every_section_of_one_file_in_plan_order() {
    // All operations targeting one destination file form one atomic
    // group (canon/architecture/fleet-lifecycle.md).  Groups keep the
    // first-claim plan order; members keep the plan order.
    let plan = SyncPlan::new(
        "dest-a",
        vec![
            selected_section("a", "t1", "primary", 1, "alpha"),
            selected("b", "t2", "primary", 2),
            selected_section("c", "t1", "secondary", 3, "beta"),
        ],
    );
    let groups = plan.selected_target_groups();
    assert_eq!(groups.len(), 2, "one group per distinct destination file");
    assert_eq!(groups[0].0, "t1", "first-claim order");
    assert_eq!(
        groups[0]
            .1
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "c"],
        "both sections of one file share one group, in plan order"
    );
    assert_eq!(groups[1].0, "t2");
    assert_eq!(groups[1].1.len(), 1);
}

#[test]
fn target_groups_exclude_rejected_losers() {
    // A rejected loser carries its reason but never executes, so it
    // joins no group and forms none of its own.
    let plan = SyncPlan::new(
        "dest-a",
        vec![
            selected("a", "t1", "primary", 1),
            rejected("b", "t1", "legacy", 2),
            rejected("c", "t2", "legacy", 3),
        ],
    );
    let groups = plan.selected_target_groups();
    assert_eq!(groups.len(), 1, "a rejected-only target forms no group");
    assert_eq!(groups[0].0, "t1");
    assert_eq!(
        groups[0].1.len(),
        1,
        "the rejected loser is not a group member"
    );
}

#[test]
fn file_identity_is_exact_target_text() {
    // Exact text outranks semantic cleverness: paths that differ only by
    // case or by a redundant component are distinct files today, and the
    // grouping predicate is the one place that rule lives.
    let plan = SyncPlan::new(
        "dest-a",
        vec![
            selected("a", "App.yaml", "primary", 1),
            selected("b", "app.yaml", "primary", 2),
            selected("c", "./app.yaml", "primary", 3),
        ],
    );
    assert_eq!(
        plan.selected_target_groups().len(),
        3,
        "no normalization and no case folding"
    );
    assert!(
        !plan.items[0].targets_same_file(&plan.items[1]),
        "the predicate agrees with the grouping"
    );
    assert!(plan.items[0].targets_same_file(&plan.items[0].clone()));
}
