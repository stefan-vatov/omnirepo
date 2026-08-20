//! Focused proof for whole-file replacement request mapping.

#![allow(dead_code, unused_imports)]

use super::{ReplacementRequest, RequestError, map_whole_file_requests};
use crate::lifecycle::sync_plan::{PlanDecision, PlanItem, SyncPlan};
use crate::platform::RelativePath;

fn selected(id: &str, target: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        source_order: 1,
        kind: crate::source::ItemKind::WholeFile,
        section: None,
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    }
}

fn rejected(id: &str, target: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: "legacy".to_owned(),
        source_path: String::new(),
        source_order: 2,
        kind: crate::source::ItemKind::WholeFile,
        section: None,
        decision: PlanDecision::Rejected {
            reason: "shadowed".to_owned(),
        },
    }
}

#[test]
fn every_whole_file_operation_yields_one_contained_request() {
    let plan = SyncPlan::new(
        "dest-a",
        vec![
            selected("item-a", "apps/app.yaml"),
            rejected("item-b", "apps/legacy.yaml"),
        ],
    );
    let requests = map_whole_file_requests(&plan, "source-1", "config-1").expect("map");
    assert_eq!(requests.len(), 1, "rejected items are skipped");
    let request = &requests[0];
    assert_eq!(request.plan_item_id, "item-a");
    assert_eq!(
        request.target,
        RelativePath::parse("apps/app.yaml").expect("path")
    );
    assert_eq!(request.source_identity, "source-1");
    assert_eq!(request.configuration_identity, "config-1");
    // The plan identity is the deterministic render, preserved verbatim.
    assert_eq!(request.plan_identity, plan.render());
}

#[test]
fn identities_are_preserved_for_journaling_and_revalidation() {
    let plan = SyncPlan::new("dest-a", vec![selected("a", "f.txt")]);
    let requests = map_whole_file_requests(&plan, "source-2", "config-2").expect("map");
    let request = &requests[0];
    assert_eq!(request.source_identity, "source-2");
    assert_eq!(request.configuration_identity, "config-2");
    assert!(!request.plan_identity.is_empty());
    // The plan render is byte-stable: revalidation can compare it exactly.
    assert_eq!(request.plan_identity, plan.render());
}

#[test]
fn section_items_are_rejected() {
    let section_item = PlanItem {
        id: "a".to_owned(),
        target: "f.txt".to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        source_order: 1,
        kind: crate::source::ItemKind::Section,
        section: Some(crate::configuration::SectionId::new("rules").expect("id")),
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    };
    let plan = SyncPlan::new("dest-a", vec![section_item]);
    let error = map_whole_file_requests(&plan, "s", "c").expect_err("section");
    assert!(matches!(error, RequestError::SectionItem { .. }), "{error}");
}

#[test]
fn empty_plans_yield_no_requests() {
    let plan = SyncPlan::new("dest-a", vec![]);
    let requests = map_whole_file_requests(&plan, "s", "c").expect("map");
    assert!(requests.is_empty());
}
