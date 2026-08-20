//! Item identity and overlap resolution by declared source order.
//!
//! The owner truth table (canon/architecture/managed-content.md):
//! duplicate declaration IDs within one source are invalid; compatible
//! cross-source overlap — the same declaration ID, the same whole-file
//! target, or the same named section on one target — resolves to the
//! earliest declared source; incompatible whole-file/section collisions
//! on one target fail before any destination mutation.  Distinct named
//! sections on one target are independent and all win.  Completion
//! timing and content never alter the winner.  Losers stay explainable:
//! each loser carries the collision kind and its declared position.

#![allow(dead_code)]

use crate::configuration::SectionId;
use std::{error::Error, fmt};

/// One item's representation kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemKind {
    WholeFile,
    Section,
}

/// Why an item lost a compatible collision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CollisionKind {
    /// The same declaration ID from a later source.
    DuplicateId,
    /// The same whole-file target from a later source.
    SameTarget,
    /// The same named section on one target from a later source.
    SameSection,
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
    /// The named section for section items; None for whole files.
    pub section: Option<SectionId>,
    /// The winning declaration index (declared order).
    pub winner: usize,
    pub losers: Vec<LoserRef>,
}

/// Resolution failures; every failure names the item.
#[derive(Debug)]
pub enum ResolutionError {
    Empty,
    /// Duplicate declaration IDs within one source are invalid.
    DuplicateIdWithinSource {
        source: String,
        id: String,
    },
    /// A whole-file item and a section item on one target are
    /// incompatible and fail before destination mutation.
    WholeVsSection {
        target: String,
        whole_file_item: String,
        section_item: String,
    },
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "no declarations were provided"),
            Self::DuplicateIdWithinSource { source, id } => write!(
                formatter,
                "source {source} declares the duplicate item id {id:?}; duplicate ids within one source are invalid"
            ),
            Self::WholeVsSection {
                target,
                whole_file_item,
                section_item,
            } => write!(
                formatter,
                "target {target} is claimed whole-file by item {whole_file_item:?} and as a section by item {section_item:?}; the collision is incompatible"
            ),
        }
    }
}
impl Error for ResolutionError {}

/// A declaration's resolved shape (id, target, section identity, declared
/// order).  The binder fills this; resolution decides the winners.
#[derive(Clone, Debug)]
pub struct ItemDeclaration {
    pub id: String,
    pub target: String,
    /// The owning source in declared precedence order.
    pub source: String,
    /// The source-relative file path that carries the payload.
    pub source_path: String,
    pub kind: ItemKind,
    /// The named section for section items; None for whole files.
    pub section: Option<SectionId>,
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
        // Duplicate IDs within one source are invalid, never resolved.
        if declared[..index]
            .iter()
            .any(|prior| prior.source == item.source && prior.id == item.id)
        {
            return Err(ResolutionError::DuplicateIdWithinSource {
                source: item.source.clone(),
                id: item.id.clone(),
            });
        }
        // Incompatibility outranks precedence: a whole-file claim beside
        // a section claim on one target fails typed even when the two
        // declarations share an ID — folding it into a duplicate-ID loser
        // would hide the incompatible collision.
        let same_target = winners
            .iter()
            .position(|winner| winner.target == item.target);
        if let Some(position) = same_target {
            let winner_kind = winners[position].kind;
            match (winner_kind, item.kind) {
                (ItemKind::WholeFile, ItemKind::Section) => {
                    return Err(ResolutionError::WholeVsSection {
                        target: item.target.clone(),
                        whole_file_item: winners[position].id.clone(),
                        section_item: item.id.clone(),
                    });
                }
                (ItemKind::Section, ItemKind::WholeFile) => {
                    return Err(ResolutionError::WholeVsSection {
                        target: item.target.clone(),
                        whole_file_item: item.id.clone(),
                        section_item: winners[position].id.clone(),
                    });
                }
                (ItemKind::WholeFile, ItemKind::WholeFile)
                | (ItemKind::Section, ItemKind::Section) => {}
            }
        }
        // The same declaration ID from a later source is the same logical
        // item: the earlier source wins.
        if let Some(winner) = winners.iter_mut().find(|winner| winner.id == item.id) {
            winner.losers.push(LoserRef {
                declaration_index: index,
                collision: CollisionKind::DuplicateId,
            });
            continue;
        }
        // Same-target rules.  Resolution keeps targets homogeneous: a
        // target is either one whole file or a set of named sections.
        if let Some(position) = same_target {
            match (winners[position].kind, item.kind) {
                (ItemKind::WholeFile, ItemKind::WholeFile) => {
                    winners[position].losers.push(LoserRef {
                        declaration_index: index,
                        collision: CollisionKind::SameTarget,
                    });
                    continue;
                }
                (ItemKind::Section, ItemKind::Section) => {
                    let same_section = winners.iter_mut().find(|winner| {
                        winner.target == item.target && winner.section == item.section
                    });
                    if let Some(winner) = same_section {
                        winner.losers.push(LoserRef {
                            declaration_index: index,
                            collision: CollisionKind::SameSection,
                        });
                        continue;
                    }
                    // A distinct named section on the same target is
                    // independent: fall through to win.
                }
                _ => unreachable!("incompatible kinds already failed typed"),
            }
        }
        winners.push(ResolvedItem {
            id: item.id.clone(),
            target: item.target.clone(),
            kind: item.kind,
            section: item.section.clone(),
            winner: index,
            losers: Vec::new(),
        });
    }
    Ok(winners)
}
