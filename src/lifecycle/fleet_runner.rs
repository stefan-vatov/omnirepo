//! Run one repository initial pass per admitted item through the fleet
//! runner.
//!
//! The bounded fleet pass executes the canonical initial pass for every
//! admitted repository: the frozen snapshot is built for the
//! destination, the pass applies the managed changes, runs the declared
//! checks in configured order, and commits and pushes only after the
//! checks pass, with at most one commit per repository per run.  An
//! unchanged repository creates no commit.  A failure never stops its
//! peers and every repository reaches exactly one result in declared
//! order.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_runner_tests;

use crate::configuration::MachineConfiguration;
use crate::lifecycle::fleet_app::{FleetComposition, FleetResponse, run_fleet_pass};
use crate::lifecycle::fleet_fanout::RepoResult;
use crate::lifecycle::fleet_snapshot::build_frozen_snapshot;
use crate::lifecycle::journal::JournalHandle;
use crate::lifecycle::single_repo_pass::run_single_repository_pass;
use crate::lifecycle::work_mapping::WorkItem;
use crate::repository::RepositorySnapshot;
use std::collections::HashMap;
use std::sync::Arc;

/// The canonical commit message for a synchronization delivery.
pub const SYNC_COMMIT_MESSAGE: &str = "chore(omnirepo): sync managed content";

/// Run the fleet initial passes over the composition.
///
/// Every admitted repository gets its frozen snapshot and exactly one
/// initial pass; the response carries one result per repository in
/// declared order.  A repository whose snapshot or pass fails is a typed
/// failure and never stops its peers.
pub fn run_fleet_initial_passes(
    journal: &JournalHandle,
    run_id: &str,
    config: &MachineConfiguration,
    plans: &[crate::lifecycle::fleet_planning::RepositoryPlan],
    composition: &FleetComposition,
    limit: usize,
) -> Result<FleetResponse, String> {
    if composition.work.is_empty() {
        return Ok(FleetResponse {
            run_id: run_id.to_owned(),
            results: Vec::new(),
            frozen_repair_inputs: Vec::new(),
        });
    }
    // Owned per-repository destinations (the runner closure is 'static).
    let destinations = config
        .repositories()
        .iter()
        .map(|destination| {
            (
                destination.id().as_str().to_owned(),
                destination.path().as_str().to_owned(),
            )
        })
        .collect::<HashMap<String, String>>();
    // Build the frozen snapshot per admitted repository first.
    let mut snapshots: HashMap<String, RepositorySnapshot> = HashMap::new();
    for item in &composition.work {
        let WorkItem::Run { repository, .. } = item else {
            continue;
        };
        let Some(working) = destinations.get(repository) else {
            continue;
        };
        let plan = plans
            .iter()
            .find(|plan| plan.repository == *repository)
            .and_then(|plan| plan.plan.as_ref().ok())
            .cloned()
            .unwrap_or_else(|| crate::lifecycle::sync_plan::SyncPlan::new(repository, Vec::new()));
        let snapshot = build_frozen_snapshot(std::path::Path::new(working), &plan)
            .map_err(|error| format!("{repository}: {error}"))?;
        snapshots.insert(repository.clone(), snapshot);
    }
    let shared = Arc::new(snapshots);
    let destinations = Arc::new(destinations);
    let journal_handle = journal.clone();
    let run_id_owned = run_id.to_owned();
    let mut lease_check = |_repository: &str| true;
    let runner = {
        let shared = Arc::clone(&shared);
        let destinations = Arc::clone(&destinations);
        move |item: &WorkItem| match item {
            WorkItem::Skip { repository, reason } => RepoResult::Skipped {
                repository: repository.clone(),
                reason: reason.clone(),
            },
            WorkItem::Run { repository, .. } => {
                let Some(working) = destinations.get(repository) else {
                    return RepoResult::Failed {
                        repository: repository.clone(),
                        reason: "the repository is not declared".to_owned(),
                    };
                };
                let Some(snapshot) = shared.get(repository) else {
                    return RepoResult::Failed {
                        repository: repository.clone(),
                        reason: "the frozen snapshot is unavailable".to_owned(),
                    };
                };
                let working = std::path::Path::new(working);
                match run_single_repository_pass(
                    working,
                    &journal_handle,
                    &run_id_owned,
                    repository,
                    snapshot,
                    SYNC_COMMIT_MESSAGE,
                ) {
                    Ok(crate::lifecycle::single_repo_pass::PassOutcome::Delivered { oid }) => {
                        RepoResult::Delivered {
                            repository: repository.clone(),
                            oid,
                        }
                    }
                    Ok(crate::lifecycle::single_repo_pass::PassOutcome::Failed { reason }) => {
                        RepoResult::Failed {
                            repository: repository.clone(),
                            reason,
                        }
                    }
                    Err(error) => RepoResult::Failed {
                        repository: repository.clone(),
                        reason: error.to_string(),
                    },
                }
            }
        }
    };
    run_fleet_pass(run_id, &composition.work, limit, &mut lease_check, runner)
        .map_err(|error| error.to_string())
}
