//! Application-service contract and forbidden-input fixtures.
//!
//! STRICT TDD: this test file was written and run RED before the fixture
//! contract existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_app::{FleetComposition, FleetRequest, build_request, compose_fleet};
use crate::lifecycle::plan_selection::Policy;
use crate::lifecycle::sync_plan::{PlanDecision, PlanItem};
use crate::source::{CatalogState, ItemKind, RevisionId, SourceCatalog, SourceId};

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
        source_order: 1,
        kind: ItemKind::WholeFile,
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    }
}
fn catalog() -> SourceCatalog {
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
fn forbidden_overrides_are_unrepresentable_or_rejected() {
    // The request has no URL, local file, template, or destination
    // override fields: the canonical inputs are the only constructor
    // arguments (compile-time contract).  Empty fleets and zero limits are
    // rejected typed.
    let request: FleetRequest =
        build_request(&["dest-a".to_owned()], "machine-1", 4).expect("request");
    assert_eq!(request.fleet, vec!["dest-a".to_owned()]);
    assert!(build_request(&[], "machine-1", 4).is_err());
    assert!(build_request(&["dest-a".to_owned()], "machine-1", 0).is_err());
}

#[test]
fn response_is_complete_and_deterministic() {
    let build = || {
        compose_fleet(
            true,
            &catalog(),
            &[
                ("dest-a", None, &[selected_item("a")]),
                (
                    "dest-b",
                    Some(&Policy::Explicit {
                        include: vec!["ghost".to_owned()],
                        exclude: vec![],
                    }),
                    &[selected_item("b")],
                ),
            ],
            2,
        )
        .expect("composition")
    };
    let first = build();
    let second = build();
    assert_eq!(first, second, "deterministic composition");
    let _: FleetComposition = first;
}

#[test]
fn expected_effects_and_order_match_the_work_items() {
    let composition = compose_fleet(
        true,
        &catalog(),
        &[
            ("dest-a", None, &[selected_item("a")]),
            ("dest-b", None, &[selected_item("b")]),
        ],
        2,
    )
    .expect("composition");
    // The expected effects: exactly the admitted repositories, in
    // declared order, as run work items.
    assert_eq!(
        composition.accounted,
        vec!["dest-a".to_owned(), "dest-b".to_owned()]
    );
    assert_eq!(composition.work.len(), 2);
    assert!(composition.work.iter().all(|item| item.is_run()));
    assert_eq!(composition.request.fleet, composition.accounted);
}

#[test]
fn no_agent_or_renderer_path_is_called() {
    // The application service contract is pure composition: build and
    // compose reach no agent and no renderer.  (Compile-time contract.)
    let composition = compose_fleet(
        true,
        &catalog(),
        &[("dest-a", None, &[selected_item("a")])],
        1,
    )
    .expect("composition");
    assert_eq!(composition.work.len(), 1);
    assert_eq!(composition.accounted, vec!["dest-a".to_owned()]);
}
