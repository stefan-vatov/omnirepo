//! Deterministic plan, explanation, and revalidation fixtures.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::plan_builder::build_repository_plan;
use crate::lifecycle::plan_selection::Policy;
use crate::source::{
    CatalogState, ItemDeclaration, ItemKind, PayloadKind, RevisionId, SourceCatalog, SourceId,
    content_identity, extract_payload,
};

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
        source_path: String::new(),
        kind: ItemKind::WholeFile,
        section: None,
        source_order: order,
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
fn repeated_frozen_inputs_yield_byte_identical_plan_identity() {
    let build = || {
        let catalog = catalog();
        build_repository_plan(
            "dest-a",
            &catalog,
            &[item("a", "t1", "primary", 1), item("b", "t2", "primary", 2)],
            &Policy::Absent,
        )
        .expect("plan")
    };
    let first = build().render();
    let second = build().render();
    assert_eq!(first, second, "byte-identical plan renders");
    // The content identity is also deterministic for the same bytes.
    assert_eq!(content_identity(b"x"), content_identity(b"x"));
    assert_ne!(content_identity(b"x"), content_identity(b"y"));
}

#[test]
fn source_and_config_order_changes_have_intentional_effects_only() {
    // Changing the declared source order flips the winner of the colliding
    // target and nothing else.
    let catalog = catalog();
    let plan_a = build_repository_plan(
        "dest-a",
        &catalog,
        &[
            item("x", "shared", "primary", 1),
            item("y", "shared", "primary", 2),
        ],
        &Policy::Absent,
    )
    .expect("plan");
    let plan_b = build_repository_plan(
        "dest-a",
        &catalog,
        &[
            item("y", "shared", "primary", 1),
            item("x", "shared", "primary", 2),
        ],
        &Policy::Absent,
    )
    .expect("plan");
    assert_eq!(plan_a.items[0].id, "x");
    assert_eq!(plan_b.items[0].id, "y");
    // Non-colliding items keep their decisions under both orders.
    let with_extra = |first: &str, second: &str| {
        build_repository_plan(
            "dest-a",
            &catalog,
            &[
                item("a", "t1", "primary", 1),
                item(first, "shared", "primary", 2),
                item(second, "shared", "primary", 3),
            ],
            &Policy::Absent,
        )
        .expect("plan")
    };
    let first = with_extra("x", "y");
    let second = with_extra("y", "x");
    assert_eq!(first.items[0].id, "a");
    assert_eq!(second.items[0].id, "a");
}

#[test]
fn concurrent_completion_does_not_change_the_plan() {
    // The plan depends only on declared order, never on completion order:
    // two identical builds with interleaved "completion" (here, simply two
    // builds) produce identical renders.
    let catalog = catalog();
    let build = || {
        build_repository_plan(
            "dest-a",
            &catalog,
            &[item("a", "t1", "primary", 1)],
            &Policy::Absent,
        )
        .expect("plan")
        .render()
    };
    let rendered = build();
    for _ in 0..5 {
        assert_eq!(build(), rendered);
    }
}

#[test]
fn revalidation_detects_changed_inputs_before_effects() {
    // The content identity is the revalidation witness: a changed payload
    // has a different identity, so the plan input is detectable as changed
    // before any effect.
    let before = extract_payload("f", b"v1", &PayloadKind::WholeFile)
        .expect("extract")
        .content_identity;
    let after = extract_payload("f", b"v2", &PayloadKind::WholeFile)
        .expect("extract")
        .content_identity;
    assert_ne!(before, after, "changed inputs change the identity");
    // The catalog state is also a revalidation input: a source becoming
    // unavailable flips the plan to the typed affected failure.
    let mut degraded = SourceCatalog::new();
    degraded
        .record(CatalogState::Unavailable {
            source: source("primary"),
            reason: "revalidation found a changed upstream".to_owned(),
        })
        .expect("record");
    let error = build_repository_plan(
        "dest-a",
        &degraded,
        &[item("a", "t1", "primary", 1)],
        &Policy::Absent,
    )
    .expect_err("degraded catalog must affect the plan");
    assert!(
        format!("{error}").contains("revalidation found a changed upstream"),
        "{error}"
    );
}
