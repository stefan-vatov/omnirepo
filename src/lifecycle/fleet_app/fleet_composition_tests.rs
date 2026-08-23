//! Focused proof for composing machine authority, source catalog,
//! policies, and plans into the fleet application.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_app::compose_fleet;
use crate::lifecycle::plan_selection::Policy;
use crate::lifecycle::sync_plan::{PlanDecision, PlanItem};
use crate::source::RevisionId;
use crate::source::{CatalogState, ItemKind, SourceCatalog, SourceId};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn selected_item(id: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: "t".to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        source_order: 1,
        kind: ItemKind::WholeFile,
        section: None,
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
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
fn invalid_machine_authority_stops_all_repositories() {
    let catalog = complete_catalog();
    let composition = compose_fleet(
        false,
        &catalog,
        &[("dest-a", None, &[selected_item("a")])],
        2,
    )
    .expect("composition");
    // Invalid machine authority: nothing is admitted, every repository is
    // in the initial accounting as affected.
    assert!(composition.work.is_empty());
    assert_eq!(composition.affected.len(), 1, "all repos are affected");
}

#[test]
fn source_and_policy_failures_are_scoped_correctly() {
    let catalog = complete_catalog();
    let composition = compose_fleet(
        true,
        &catalog,
        &[
            ("dest-a", None, &[selected_item("a")]),
            (
                "dest-b",
                Some(&Policy::Explicit {
                    all: false,
                    include: vec!["ghost".to_owned()],
                    exclude: vec![],
                }),
                &[selected_item("a")],
            ),
        ],
        2,
    )
    .expect("composition");
    // dest-a is admitted as a run; dest-b is skipped by its own policy
    // failure — both are scheduler work, scoped correctly.
    assert_eq!(composition.work.len(), 2);
    assert!(composition.work[0].is_run());
    assert!(composition.work[1].is_skip());
    assert_eq!(composition.skipped.len(), 1);
    assert_eq!(composition.skipped[0].0, "dest-b");
}

#[test]
fn every_determinable_repository_enters_initial_accounting() {
    let catalog = complete_catalog();
    let composition = compose_fleet(
        true,
        &catalog,
        &[
            ("dest-a", None, &[selected_item("a")]),
            ("dest-b", None, &[selected_item("b")]),
        ],
        1,
    )
    .expect("composition");
    assert_eq!(composition.work.len(), 2, "both repos admitted");
    assert_eq!(composition.accounted.len(), 2);
    assert_eq!(composition.request.fleet.len(), 2);
}

#[test]
fn empty_declarations_yield_an_empty_composition() {
    let catalog = complete_catalog();
    let composition = compose_fleet(true, &catalog, &[], 2).expect("composition");
    assert!(composition.work.is_empty());
    assert!(composition.request.fleet.is_empty());
}
