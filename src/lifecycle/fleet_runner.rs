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
    // Owned per-repository destinations and plans (the runner closure is
    // 'static).  The frozen snapshot is built lazily per admitted
    // repository inside the runner: a snapshot failure fails that
    // repository only and never aborts the fleet pass.
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
    let sources = config
        .sources()
        .iter()
        .filter_map(|source| match source.location() {
            crate::configuration::SourceLocation::Local(path) => Some((
                source.id().as_str().to_owned(),
                std::path::PathBuf::from(path.as_str()),
            )),
            crate::configuration::SourceLocation::Remote(_) => None,
        })
        .collect::<HashMap<String, std::path::PathBuf>>();
    let plans = plans
        .iter()
        .map(|plan| {
            (
                plan.repository.clone(),
                (
                    plan.plan.as_ref().ok().cloned().unwrap_or_else(|| {
                        crate::lifecycle::sync_plan::SyncPlan::new(&plan.repository, Vec::new())
                    }),
                    plan.checks.clone(),
                ),
            )
        })
        .collect::<HashMap<
            String,
            (
                crate::lifecycle::sync_plan::SyncPlan,
                Vec<crate::repository::VerificationCommand>,
            ),
        >>();
    let destinations = Arc::new(destinations);
    let sources = Arc::new(sources);
    let plans = Arc::new(plans);
    let journal_handle = journal.clone();
    let run_id_owned = run_id.to_owned();
    let mut lease_check = |_repository: &str| true;
    let runner = {
        let destinations = Arc::clone(&destinations);
        let sources = Arc::clone(&sources);
        let plans = Arc::clone(&plans);
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
                let working = std::path::Path::new(working);
                let (plan, checks) = plans.get(repository).cloned().unwrap_or_else(|| {
                    (
                        crate::lifecycle::sync_plan::SyncPlan::new(repository, Vec::new()),
                        Vec::new(),
                    )
                });
                let snapshot = match build_frozen_snapshot(working, &plan) {
                    Ok(snapshot) => snapshot,
                    Err(reason) => {
                        return RepoResult::Failed {
                            repository: repository.clone(),
                            reason,
                        };
                    }
                };
                match run_single_repository_pass(
                    working,
                    &journal_handle,
                    &run_id_owned,
                    repository,
                    &snapshot,
                    &checks,
                    &plan,
                    &sources,
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
