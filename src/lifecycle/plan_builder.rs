//! Per-repository plan building with collision and authority resolution.
//!
//! A complete catalog and item resolution produce the plan; an affected
//! repository (unavailable or shadowed source) fails with the named
//! source and item reasons.  Collision behavior follows the identity
//! policy (declared order truth table); no content heuristic and no
//! completion order participate.

#![allow(dead_code)]

use super::plan_selection::{Policy, SelectionDecision, select_items};
use super::sync_plan::{PlanDecision, PlanItem, SyncPlan, validate_plan};
use crate::source::{
    CatalogState, ItemDeclaration, ResolvedItem, SourceCatalog, SourceId, resolve_items,
};
use std::{error::Error, fmt};

/// Plan-build failures; every failure names the source and item.
#[derive(Debug)]
pub enum PlanBuildError {
    Affected {
        source: String,
        item: Option<String>,
        reason: String,
    },
    Resolution {
        reason: String,
    },
    Selection {
        reason: String,
    },
}

impl fmt::Display for PlanBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Affected {
                source,
                item,
                reason,
            } => match item {
                Some(item) => write!(
                    formatter,
                    "repository plan is affected by source {source} item {item}: {reason}"
                ),
                None => write!(
                    formatter,
                    "repository plan is affected by source {source}: {reason}"
                ),
            },
            Self::Resolution { reason } => write!(formatter, "plan resolution failed: {reason}"),
            Self::Selection { reason } => write!(formatter, "plan selection failed: {reason}"),
        }
    }
}
impl Error for PlanBuildError {}

/// Build the repository plan.  The catalog must be in declared order; the
/// items in declared order; the policy explicit or absent.
pub fn build_repository_plan(
    destination: &str,
    catalog: &SourceCatalog,
    declared: &[ItemDeclaration],
    policy: &Policy,
) -> Result<SyncPlan, PlanBuildError> {
    // Source availability: a complete source proceeds; an unavailable or
    // shadowed source affects the plan with the named reason.
    for item in declared {
        let state = catalog.entries().iter().find(|entry| {
            catalog_source(entry) == Some(&SourceId::new(&item.source).expect("source id"))
        });
        match state {
            Some(CatalogState::Complete { .. }) => {}
            Some(CatalogState::Unavailable { reason, .. }) => {
                return Err(PlanBuildError::Affected {
                    source: item.source.clone(),
                    item: Some(item.id.clone()),
                    reason: reason.clone(),
                });
            }
            Some(CatalogState::Shadowed { by, .. }) => {
                return Err(PlanBuildError::Affected {
                    source: item.source.clone(),
                    item: Some(item.id.clone()),
                    reason: format!("shadowed by the higher-precedence source {}", by.as_str()),
                });
            }
            None => {
                return Err(PlanBuildError::Affected {
                    source: item.source.clone(),
                    item: Some(item.id.clone()),
                    reason: "the source is not declared".to_owned(),
                });
            }
        }
    }
    // Collision behavior follows the identity policy: the declared-order
    // truth table; no content heuristic participates.
    let resolved = resolve_items(declared).map_err(|error| PlanBuildError::Resolution {
        reason: error.to_string(),
    })?;
    let plan_items = resolved
        .iter()
        .map(|winner: &ResolvedItem| PlanItem {
            id: winner.id.clone(),
            target: winner.target.clone(),
            source: declared[winner.winner].source.clone(),
            source_path: declared[winner.winner].source_path.clone(),
            source_order: declared[winner.winner].source_order,
            kind: winner.kind,
            section: winner.section.clone(),
            decision: PlanDecision::Selected {
                reason: "declared winner".to_owned(),
            },
        })
        .collect::<Vec<_>>();
    // Explicit policy selection (absent policy already selected).
    let selections =
        select_items(&plan_items, policy).map_err(|error| PlanBuildError::Selection {
            reason: error.to_string(),
        })?;
    let mut selected = selections
        .into_iter()
        .filter(|selection| matches!(selection.decision, SelectionDecision::Selected { .. }))
        .map(|selection| selection.item)
        .collect::<Vec<_>>();
    // Losers stay visible: every shadowed declaration appears in the plan
    // as a rejected item naming its winner, never as a silent drop.
    for winner in &resolved {
        for loser in &winner.losers {
            let declaration = &declared[loser.declaration_index];
            selected.push(PlanItem {
                id: declaration.id.clone(),
                target: declaration.target.clone(),
                source: declaration.source.clone(),
                source_path: declaration.source_path.clone(),
                source_order: declaration.source_order,
                kind: declaration.kind,
                section: declaration.section.clone(),
                decision: PlanDecision::Rejected {
                    reason: format!(
                        "{:?} collision: shadowed by item {} from source {}",
                        loser.collision, winner.id, declared[winner.winner].source
                    ),
                },
            });
        }
    }
    let plan = SyncPlan::new(destination, selected);
    validate_plan(&plan).map_err(|error| PlanBuildError::Selection {
        reason: error.to_string(),
    })?;
    Ok(plan)
}

fn catalog_source(state: &CatalogState) -> Option<&SourceId> {
    match state {
        CatalogState::Complete { source, .. }
        | CatalogState::Shadowed { source, .. }
        | CatalogState::Unavailable { source, .. } => Some(source),
    }
}

#[cfg(test)]
mod plan_builder_tests;

#[cfg(test)]
mod plan_fixture_tests;
