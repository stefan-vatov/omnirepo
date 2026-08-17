//! Focused proof for item identity and overlap resolution.

#![allow(dead_code, unused_imports)]

use super::{
    CollisionKind, ItemDeclaration, ItemKind, ResolutionError, ResolvedItem, resolve_items,
};

fn whole(id: &str, target: &str, order: usize) -> ItemDeclaration {
    ItemDeclaration {
        id: id.to_owned(),
        target: target.to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        kind: ItemKind::WholeFile,
        section: None,
        source_order: order,
    }
}

fn section(id: &str, target: &str, start: u64, end: u64, order: usize) -> ItemDeclaration {
    ItemDeclaration {
        id: id.to_owned(),
        target: target.to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        kind: ItemKind::Section,
        section: Some((start, end)),
        source_order: order,
    }
}

#[test]
fn duplicate_ids_follow_declared_order() {
    let items = resolve_items(&[whole("a", "t1", 1), whole("a", "t2", 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "a");
    assert_eq!(items[0].winner, 0);
    assert_eq!(
        items[0].losers,
        vec![super::LoserRef {
            declaration_index: 1,
            collision: CollisionKind::DuplicateId,
        }]
    );
}

#[test]
fn same_target_and_whole_vs_section_follow_declared_order() {
    let items = resolve_items(&[whole("a", "t", 1), whole("b", "t", 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].winner, 0);
    assert_eq!(items[0].losers[0].collision, CollisionKind::SameTarget);

    let items = resolve_items(&[whole("a", "t", 1), section("b", "t", 1, 5, 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, ItemKind::WholeFile);
    assert_eq!(items[0].losers[0].collision, CollisionKind::WholeVsSection);

    // The reverse order: the section is declared first but the whole file
    // still covers the target.
    let items = resolve_items(&[section("a", "t", 1, 5, 1), whole("b", "t", 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, ItemKind::WholeFile);
    assert_eq!(items[0].winner, 1);
    assert_eq!(items[0].losers[0].collision, CollisionKind::WholeVsSection);
}

#[test]
fn overlapping_sections_collide_and_disjoint_sections_are_independent() {
    let items =
        resolve_items(&[section("a", "t", 1, 5, 1), section("b", "t", 4, 9, 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].losers[0].collision,
        CollisionKind::MultiSectionOverlap
    );

    let items =
        resolve_items(&[section("a", "t", 1, 3, 1), section("b", "t", 5, 9, 2)]).expect("resolve");
    assert_eq!(items.len(), 2, "disjoint sections are independent");
    assert_eq!(items[0].id, "a");
    assert_eq!(items[1].id, "b");
}

#[test]
fn cross_source_collisions_follow_declared_order() {
    // Two sources declare the same target; the earlier declaration wins.
    let items =
        resolve_items(&[whole("src1-item", "t", 1), whole("src2-item", "t", 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].winner, 0);
    assert_eq!(items[0].losers[0].collision, CollisionKind::SameTarget);
}

#[test]
fn completion_timing_never_alters_the_winner() {
    // The same inputs with a different declaration order produce the
    // declared winner (the earlier declaration), regardless of which
    // source completes first.
    let first = resolve_items(&[whole("x", "t", 1), whole("y", "t", 2)]).expect("resolve");
    assert_eq!(first[0].winner, 0);
    let reversed = resolve_items(&[whole("y", "t", 1), whole("x", "t", 2)]).expect("resolve");
    assert_eq!(reversed[0].winner, 0);
    assert_eq!(reversed[0].id, "y");
}

#[test]
fn empty_input_fails_typed() {
    let error = resolve_items(&[]).expect_err("empty");
    assert!(matches!(error, ResolutionError::Empty), "{error}");
}
