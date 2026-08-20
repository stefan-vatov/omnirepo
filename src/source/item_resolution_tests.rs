//! Focused proof for item identity and overlap resolution.

#![allow(dead_code, unused_imports)]

use super::{
    CollisionKind, ItemDeclaration, ItemKind, ResolutionError, ResolvedItem, resolve_items,
};
use crate::configuration::SectionId;

fn whole_from(id: &str, source: &str, target: &str, order: usize) -> ItemDeclaration {
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

fn whole(id: &str, target: &str, order: usize) -> ItemDeclaration {
    whole_from(id, "primary", target, order)
}

fn section_from(
    id: &str,
    source: &str,
    target: &str,
    section: &str,
    order: usize,
) -> ItemDeclaration {
    ItemDeclaration {
        id: id.to_owned(),
        target: target.to_owned(),
        source: source.to_owned(),
        source_path: String::new(),
        kind: ItemKind::Section,
        section: Some(SectionId::new(section).expect("valid id")),
        source_order: order,
    }
}

fn section(id: &str, target: &str, name: &str, order: usize) -> ItemDeclaration {
    section_from(id, "primary", target, name, order)
}

#[test]
fn duplicate_ids_within_one_source_are_invalid() {
    let error = resolve_items(&[whole("a", "t1", 1), whole("a", "t2", 2)]).expect_err("duplicate");
    assert!(
        matches!(error, ResolutionError::DuplicateIdWithinSource { .. }),
        "{error}"
    );
}

#[test]
fn duplicate_ids_across_sources_follow_declared_order() {
    let items = resolve_items(&[
        whole_from("a", "source-1", "t1", 1),
        whole_from("a", "source-2", "t2", 2),
    ])
    .expect("resolve");
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
fn same_whole_file_target_follows_declared_order() {
    let items = resolve_items(&[whole("a", "t", 1), whole("b", "t", 2)]).expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].winner, 0);
    assert_eq!(items[0].losers[0].collision, CollisionKind::SameTarget);
}

#[test]
fn whole_vs_section_on_one_target_is_incompatible_in_either_order() {
    let error =
        resolve_items(&[whole("a", "t", 1), section("b", "t", "s", 2)]).expect_err("incompatible");
    assert!(
        matches!(error, ResolutionError::WholeVsSection { .. }),
        "{error}"
    );
    let error =
        resolve_items(&[section("a", "t", "s", 1), whole("b", "t", 2)]).expect_err("incompatible");
    assert!(
        matches!(error, ResolutionError::WholeVsSection { .. }),
        "{error}"
    );
}

#[test]
fn same_section_collides_and_distinct_sections_are_independent() {
    // The same named section claimed twice: the earlier declaration wins.
    let items = resolve_items(&[
        section_from("a", "source-1", "t", "rules", 1),
        section_from("b", "source-2", "t", "rules", 2),
    ])
    .expect("resolve");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].losers[0].collision, CollisionKind::SameSection);

    // Distinct named sections share the destination file independently.
    let items = resolve_items(&[
        section_from("a", "source-1", "t", "rules-a", 1),
        section_from("b", "source-2", "t", "rules-b", 2),
    ])
    .expect("resolve");
    assert_eq!(items.len(), 2, "distinct sections are independent");
    assert_eq!(items[0].id, "a");
    assert_eq!(items[1].id, "b");
}

#[test]
fn cross_source_collisions_follow_declared_order() {
    // Two sources declare the same target; the earlier declaration wins.
    let items = resolve_items(&[
        whole_from("src1-item", "source-1", "t", 1),
        whole_from("src2-item", "source-2", "t", 2),
    ])
    .expect("resolve");
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

#[test]
fn a_shared_id_never_masks_an_incompatible_collision() {
    // The same declaration ID with incompatible kinds on one target
    // fails typed; it never folds into a duplicate-id loser.
    let error = resolve_items(&[
        section_from("cfg", "source-1", "t", "rules", 1),
        whole_from("cfg", "source-2", "t", 2),
    ])
    .expect_err("incompatible");
    assert!(
        matches!(error, ResolutionError::WholeVsSection { .. }),
        "{error}"
    );
}
