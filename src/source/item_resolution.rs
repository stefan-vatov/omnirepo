//! Item identity and overlap resolution by declared source order.
//!
//! The owner truth table: the earliest declared source wins every
//! collision.  Duplicate IDs, the same target, whole-vs-section, and
//! overlapping multi-section items resolve by declared order; cross-source
//! collisions follow the same precedence.  Disjoint sections on one target
//! are independent and both win.  Completion timing and content never alter
//! the winner.  Losers stay explainable: each loser carries the collision
//! kind and its declared position.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// One item's representation kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemKind {
    WholeFile,
    Section,
}

/// Why an item lost a collision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CollisionKind {
    DuplicateId,
    SameTarget,
    WholeVsSection,
    MultiSectionOverlap,
}

/// A losing declaration with its explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoserRef {
    pub declaration_index: usize,
    pub collision: CollisionKind,
}

/// One resolved item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedItem {
    pub id: String,
    pub target: String,
    pub kind: ItemKind,
    pub section: Option<(u64, u64)>,
    /// The winning declaration index (declared order).
    pub winner: usize,
    pub losers: Vec<LoserRef>,
}

/// Resolution failures.
#[derive(Debug)]
pub enum ResolutionError {
    Empty,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "no declarations were provided"),
        }
    }
}
impl Error for ResolutionError {}

/// A declaration's resolved shape (id, target, section bounds, declared
/// order).  The extractor fills this; resolution decides the winners.
#[derive(Clone, Debug)]
pub struct ItemDeclaration {
    pub id: String,
    pub target: String,
    pub kind: ItemKind,
    pub section: Option<(u64, u64)>,
    pub source_order: usize,
}

/// Resolve the declared items by the owner truth table.  `declared` must
/// be in declared source order (the parser preserves it).
pub fn resolve_items(declared: &[ItemDeclaration]) -> Result<Vec<ResolvedItem>, ResolutionError> {
    if declared.is_empty() {
        return Err(ResolutionError::Empty);
    }
    let mut winners: Vec<ResolvedItem> = Vec::new();
    for (index, item) in declared.iter().enumerate() {
        let duplicate = winners.iter_mut().find(|w| w.id == item.id);
        if let Some(winner) = duplicate {
            winner.losers.push(LoserRef {
                declaration_index: index,
                collision: CollisionKind::DuplicateId,
            });
            continue;
        }
        let collision_position = winners
            .iter()
            .position(|w| w.target == item.target && collides(w, item));
        if let Some(position) = collision_position {
            if item.kind == ItemKind::WholeFile && winners[position].kind == ItemKind::Section {
                // The whole file beats an earlier section on the same
                // target: the section is demoted and becomes an explained
                // loser of the whole-file item.
                let demoted = winners.remove(position);
                let mut losers = vec![LoserRef {
                    declaration_index: demoted.winner,
                    collision: CollisionKind::WholeVsSection,
                }];
                losers.extend(demoted.losers.iter().cloned());
                winners.push(ResolvedItem {
                    id: item.id.clone(),
                    target: item.target.clone(),
                    kind: item.kind,
                    section: item.section,
                    winner: index,
                    losers,
                });
                continue;
            }
            let (winner_kind, winner_section) = {
                let winner = &winners[position];
                (winner.kind, winner.section)
            };
            winners[position].losers.push(LoserRef {
                declaration_index: index,
                collision: classify_target_collision(
                    winner_kind,
                    item.kind,
                    winner_section,
                    item.section,
                ),
            });
            continue;
        }
        winners.push(ResolvedItem {
            id: item.id.clone(),
            target: item.target.clone(),
            kind: item.kind,
            section: item.section,
            winner: index,
            losers: Vec::new(),
        });
    }
    Ok(winners)
}

/// Two items on the same target collide unless both are disjoint sections.
fn collides(winner: &ResolvedItem, challenger: &ItemDeclaration) -> bool {
    if winner.kind != ItemKind::Section || challenger.kind != ItemKind::Section {
        return true;
    }
    match (winner.section, challenger.section) {
        (Some((a_start, a_end)), Some((b_start, b_end))) => a_start <= b_end && b_start <= a_end,
        _ => true,
    }
}

/// Classify a colliding same-target pair.  A whole-file item always beats a
/// section on the same target.
fn classify_target_collision(
    winner_kind: ItemKind,
    challenger_kind: ItemKind,
    _winner_section: Option<(u64, u64)>,
    _challenger_section: Option<(u64, u64)>,
) -> CollisionKind {
    match (winner_kind, challenger_kind) {
        (ItemKind::WholeFile, ItemKind::WholeFile) => CollisionKind::SameTarget,
        (ItemKind::WholeFile, ItemKind::Section) => CollisionKind::WholeVsSection,
        (ItemKind::Section, ItemKind::WholeFile) => CollisionKind::WholeVsSection,
        (ItemKind::Section, ItemKind::Section) => CollisionKind::MultiSectionOverlap,
    }
}
