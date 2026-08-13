//! Focused proof for per-repository plan building.

#![allow(dead_code, unused_imports)]

use super::{PlanBuildError, build_repository_plan};
use crate::lifecycle::plan_selection::Policy;
use crate::lifecycle::sync_plan::PlanDecision;
use crate::source::{CatalogState, ItemDeclaration, ItemKind, RevisionId, SourceCatalog, SourceId};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn item(id: &str, target: &str, source: &str, order: usize) -> ItemDeclaration {
    ItemDeclaration {
        id: id.to_owned(),
        target: target.to_owned(),
        source: source.to_owned(),
        kind: ItemKind::WholeFile,
        section: None,
        source_order: order,
    }
}

fn complete_catalog() -> SourceCatalog {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: source("primary"),
            revision: revision("rev-1"),
        })
        .expect("record");
    catalog
}

#[test]
fn complete_plans_proceed() {
    let catalog = complete_catalog();
    let plan = build_repository_plan(
        "dest-a",
        &catalog,
        &[item("a", "t1", "primary", 1)],
        &Policy::Absent,
    )
    .expect("plan");
    assert_eq!(plan.destination, "dest-a");
    assert_eq!(plan.items.len(), 1);
    assert!(matches!(
        plan.items[0].decision,
        PlanDecision::Selected { .. }
    ));
}

#[test]
fn affected_plans_fail_with_named_source_and_item() {
    let mut catalog = complete_catalog();
    catalog
        .record(CatalogState::Unavailable {
            source: source("broken"),
            reason: "acquisition failed".to_owned(),
        })
        .expect("record");
    let error = build_repository_plan(
        "dest-a",
        &catalog,
        &[item("a", "t1", "broken", 2)],
        &Policy::Absent,
    )
    .expect_err("affected");
    match error {
        PlanBuildError::Affected {
            source,
            item,
            reason,
        } => {
            assert_eq!(source, "broken");
            assert_eq!(item.as_deref(), Some("a"));
            assert!(reason.contains("acquisition failed"), "{reason}");
        }
        other => panic!("expected affected, got {other:?}"),
    }
}

#[test]
fn shadowed_sources_affect_the_plan_with_reason() {
    let mut catalog = complete_catalog();
    catalog
        .record(CatalogState::Shadowed {
            source: source("legacy"),
            by: source("primary"),
        })
        .expect("record");
    let error = build_repository_plan(
        "dest-a",
        &catalog,
        &[item("a", "t1", "legacy", 2)],
        &Policy::Absent,
    )
    .expect_err("shadowed");
    assert!(matches!(error, PlanBuildError::Affected { .. }), "{error}");
}

#[test]
fn collision_behavior_follows_identity_policy_only() {
    let catalog = complete_catalog();
    // Two items on the same target: the declared order decides; content
    // never participates (the bytes are identical in both orders).
    let first = build_repository_plan(
        "dest-a",
        &catalog,
        &[item("a", "t", "primary", 1), item("b", "t", "primary", 2)],
        &Policy::Absent,
    )
    .expect("plan");
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, "a");
    let reversed = build_repository_plan(
        "dest-a",
        &catalog,
        &[item("b", "t", "primary", 1), item("a", "t", "primary", 2)],
        &Policy::Absent,
    )
    .expect("plan");
    assert_eq!(reversed.items[0].id, "b");
}

#[test]
fn explicit_policy_filters_the_plan() {
    let catalog = complete_catalog();
    let plan = build_repository_plan(
        "dest-a",
        &catalog,
        &[item("a", "t1", "primary", 1), item("b", "t2", "primary", 2)],
        &Policy::Explicit {
            include: vec!["a".to_owned()],
            exclude: vec![],
        },
    )
    .expect("plan");
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].id, "a");
}
