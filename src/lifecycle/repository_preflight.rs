//! Independent repository-policy and plan preflight.
//!
//! Every machine-declared repository receives a ready plan or a failed
//! preflight.  A truly absent policy infers (the canonical default);
//! a present-but-invalid policy fails only that repository; all failures
//! aggregate deterministically in declared order.

#![allow(dead_code)]

use super::plan_selection::{Policy, SelectionError, select_items};
use super::sync_plan::{PlanDecision, PlanItem, SyncPlan};

/// One repository's preflight outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepoPreflight {
    /// The repository has a ready plan (explicit or inferred).
    ReadyPlan { repository: String },
    /// The repository failed preflight with deterministic reasons.
    Failed {
        repository: String,
        reasons: Vec<String>,
    },
}

/// Run the per-repository preflight.
///
/// `declared` is the machine-declared repository list with its optional
/// explicit policy and items.  A `None` policy infers; an explicit policy
/// that fails selection fails only that repository.
pub fn preflight_repositories(
    declared: &[(&str, Option<&Policy>, &[PlanItem])],
) -> Vec<RepoPreflight> {
    let mut outcomes = Vec::new();
    for (repository, policy, items) in declared {
        let decision = match policy {
            None => {
                // True absence infers: the canonical default selects every
                // declared winner.
                Ok(items
                    .iter()
                    .filter(|item| matches!(item.decision, PlanDecision::Selected { .. }))
                    .cloned()
                    .collect::<Vec<_>>())
            }
            Some(policy) => select_items(items, policy).map(|selections| {
                selections
                    .into_iter()
                    .filter(|selection| {
                        matches!(
                            selection.decision,
                            super::plan_selection::SelectionDecision::Selected { .. }
                        )
                    })
                    .map(|selection| selection.item)
                    .collect::<Vec<_>>()
            }),
        };
        match decision {
            Ok(selected) => {
                let plan = SyncPlan::new(*repository, selected);
                let _ = plan;
                outcomes.push(RepoPreflight::ReadyPlan {
                    repository: (*repository).to_owned(),
                });
            }
            Err(error) => outcomes.push(RepoPreflight::Failed {
                repository: (*repository).to_owned(),
                reasons: vec![error.to_string()],
            }),
        }
    }
    outcomes
}

/// Aggregate every failure deterministically (declared order preserved).
pub fn aggregate_failures(outcomes: &[RepoPreflight]) -> Vec<(String, Vec<String>)> {
    outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            RepoPreflight::Failed {
                repository,
                reasons,
            } => Some((repository.clone(), reasons.clone())),
            RepoPreflight::ReadyPlan { .. } => None,
        })
        .collect()
}

/// Map a selection failure to its stable preflight reason.
pub fn selection_failure_reason(error: &SelectionError) -> String {
    match error {
        SelectionError::UnknownSelector { selector } => {
            format!("selector {selector:?} matches no declared item")
        }
        SelectionError::ConflictingSelector { id } => {
            format!("item {id} is both included and excluded")
        }
    }
}

#[cfg(test)]
mod repository_preflight_tests;
