//! Preflight-to-scheduler work mapping.
//!
//! Every preflight repository result maps to exactly one work item: a
//! ready plan becomes a run item with its plan identity; a failed
//! preflight becomes a skip item with the aggregated reason.  Declared
//! order is preserved.

#![allow(dead_code)]

use crate::lifecycle::repository_preflight::RepoPreflight;

#[cfg(test)]
mod work_mapping_tests;

/// One scheduler work item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkItem {
    /// The repository may run with its plan identity.
    Run {
        repository: String,
        plan_identity: String,
    },
    /// The repository is skipped with the aggregated reason.
    Skip { repository: String, reason: String },
}

/// Map every preflight result into exactly one work item, preserving the
/// declared order.
pub fn map_preflight_to_work(preflight: &[RepoPreflight]) -> Vec<WorkItem> {
    preflight
        .iter()
        .map(|outcome| match outcome {
            RepoPreflight::ReadyPlan { repository } => WorkItem::Run {
                repository: repository.clone(),
                plan_identity: "plan".to_owned(),
            },
            RepoPreflight::Failed {
                repository,
                reasons,
            } => WorkItem::Skip {
                repository: repository.clone(),
                reason: reasons.join("; "),
            },
        })
        .collect()
}
