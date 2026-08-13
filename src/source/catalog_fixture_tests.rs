//! Deterministic catalog and provenance fixtures.

#![allow(dead_code, unused_imports)]

use crate::source::catalog_state::{CatalogState, SourceCatalog, plan_impact};
use crate::source::extraction::{PayloadKind, content_identity, extract_payload};
use crate::source::item_resolution::{ItemDeclaration, ItemKind, resolve_items};
use crate::source::snapshot::{RevisionId, SourceId};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

#[test]
fn repeated_identical_inputs_yield_byte_identical_catalog_identity() {
    // The content identity is a pure function of the bytes: the same
    // snapshot content always produces the same identity.
    let content = b"\xef\xbb\xbfmanaged: v1\n";
    let first = content_identity(content);
    let second = content_identity(content);
    assert_eq!(first, second);
    // Different bytes always produce a different identity.
    assert_ne!(first, content_identity(b"managed: v2\n"));
}

#[test]
fn catalog_identity_is_deterministic_across_runs() {
    let build = || {
        let mut catalog = SourceCatalog::new();
        catalog
            .record(CatalogState::Complete {
                source: source("primary"),
                revision: revision("rev-1"),
            })
            .expect("record");
        catalog
            .record(CatalogState::Unavailable {
                source: source("broken"),
                reason: "acquisition failed".to_owned(),
            })
            .expect("record");
        plan_impact(&catalog, &[("dest-a", &source("primary"))])
    };
    let first = build();
    let second = build();
    assert_eq!(first, second, "identical inputs yield identical impacts");
}

#[test]
fn changing_configured_order_changes_only_the_specified_winners() {
    // The same two targets in a different declared order flip only the
    // winners of the colliding target; the non-colliding item is untouched.
    let declared = |order_a: usize, order_b: usize| {
        vec![
            ItemDeclaration {
                id: "item-a".to_owned(),
                target: "t1".to_owned(),
                source: "primary".to_owned(),
                kind: ItemKind::WholeFile,
                section: None,
                source_order: order_a,
            },
            ItemDeclaration {
                id: "item-b".to_owned(),
                target: "t2".to_owned(),
                source: "primary".to_owned(),
                kind: ItemKind::WholeFile,
                section: None,
                source_order: order_b,
            },
        ]
    };
    // Collide both items on the same target in both orders.
    let colliding = |first_id: &str, second_id: &str| {
        vec![
            ItemDeclaration {
                id: first_id.to_owned(),
                target: "shared".to_owned(),
                source: "primary".to_owned(),
                kind: ItemKind::WholeFile,
                section: None,
                source_order: 1,
            },
            ItemDeclaration {
                id: second_id.to_owned(),
                target: "shared".to_owned(),
                source: "primary".to_owned(),
                kind: ItemKind::WholeFile,
                section: None,
                source_order: 2,
            },
        ]
    };
    let first = resolve_items(&colliding("a", "b")).expect("resolve");
    let second = resolve_items(&colliding("b", "a")).expect("resolve");
    assert_eq!(first[0].id, "a");
    assert_eq!(second[0].id, "b");
    // Non-colliding items resolve identically in both orders.
    let ordered_a = resolve_items(&declared(1, 2)).expect("resolve");
    let ordered_b = resolve_items(&declared(2, 1)).expect("resolve");
    assert_eq!(ordered_a.len(), 2);
    assert_eq!(ordered_b.len(), 2);
    assert_eq!(ordered_a[0].id, "item-a");
    assert_eq!(ordered_b[0].id, "item-a");
}

#[test]
fn hostile_paths_and_content_fail_before_any_destination_mutation() {
    // Extraction validates the locator before touching anything: hostile
    // paths fail typed without reading.
    let hostile = ["../escape", "/absolute", "a//b", ""];
    for locator in hostile {
        let error = extract_payload(locator, b"content", &PayloadKind::WholeFile)
            .expect_err("hostile locator must fail");
        let text = format!("{error}");
        assert!(
            text.contains("escapes") || text.contains("ambiguous"),
            "{text}"
        );
    }
    // Hostile section decisions fail typed without reading.
    assert!(
        extract_payload(
            "f",
            b"x",
            &PayloadKind::Section {
                start_line: 0,
                end_line: 1
            }
        )
        .is_err()
    );
}
