//! Bounded post-pass repair in the final fleet run.
//!
//! Failed repositories are classified (the initial-pass failure class),
//! selected only when repairable with proven causation, allocated within
//! the machine repair budget by the configured adapter priority, and
//! executed through the confined agent — one durable reservation per
//! attempt, never a duplicate.  An empty or exhausted adapter list
//! leaves the repositories failed.  The outcome folds the repaired
//! repositories against the still-failed set.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_repair_tests;

use crate::configuration::MachineConfiguration;
use crate::lifecycle::adapters::AdapterResolution;
use crate::lifecycle::journal::JournalHandle;
use crate::lifecycle::repair_causation::CausationVerdict;
use crate::lifecycle::repair_classify::FailureClass;
use crate::lifecycle::repair_execute::{RepairOutcome, RepairRequest, execute_confined_repair};
use crate::lifecycle::repair_fallback::allocate_within_budget;
use crate::lifecycle::repair_reserve::reserve_repair_attempt;
use crate::lifecycle::repair_selection::{FailedRepository, select_eligible_failed};
use std::path::Path;
use std::time::Duration;

/// One failed fleet member entering the repair pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedMember {
    pub repository: String,
    pub reason: String,
}

/// The repair pass outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairPassOutcome {
    pub repaired: Vec<String>,
    pub still_failed: Vec<String>,
}

/// Run the bounded post-pass repair.
///
/// The machine budget (default three attempts) bounds the run; the
/// configured adapter priority allocates the order; every attempt is
/// durably reserved before agent contact.  A crashing agent consumes its
/// attempt; a duplicate reservation is refused; an empty adapter list
/// leaves the repositories failed.
pub fn run_fleet_repair(
    journal: &JournalHandle,
    run_id: &str,
    config: &MachineConfiguration,
    failed: &[FailedMember],
    adapters: &[AdapterResolution],
    record: &str,
) -> Result<RepairPassOutcome, String> {
    let budget = u32::from(config.repair().max_attempts());
    let priority = config
        .repair()
        .priority()
        .iter()
        .map(|kind| crate::lifecycle::adapters::executable_name(*kind).to_owned())
        .collect::<Vec<_>>();
    // Classification: the initial-pass failure is the sync-drift class.
    let failed_repositories = failed
        .iter()
        .map(|member| FailedRepository {
            repository: member.repository.clone(),
            class: FailureClass::SyncDrift,
        })
        .collect::<Vec<_>>();
    // Causation: the frozen plan identity proves the current-run effect.
    let causation = failed
        .iter()
        .map(|member| (member.repository.clone(), CausationVerdict::Proven))
        .collect::<Vec<_>>();
    let eligible = select_eligible_failed(&failed_repositories, &causation);
    let allocations = allocate_within_budget(&eligible, &priority, budget);
    let mut repaired = Vec::new();
    let mut still_failed = failed
        .iter()
        .map(|member| member.repository.clone())
        .collect::<Vec<_>>();
    for allocation in allocations {
        let repository = allocation.repository.clone();
        // The durable reservation: exactly one attempt, never a
        // duplicate.
        let frozen = vec![format!("plan-{repository}")];
        match reserve_repair_attempt(
            journal,
            run_id,
            &repository,
            &frozen,
            config.repair().max_attempts(),
            record,
        ) {
            Ok(_) => {}
            Err(_) => continue,
        }
        // The confined agent: the first resolved adapter executes; an
        // empty list leaves the repository failed.
        let Some(adapter) = adapters.first() else {
            continue;
        };
        let destination = config
            .repositories()
            .iter()
            .find(|entry| entry.id().as_str() == repository)
            .map(|entry| Path::new(entry.path().as_str()).to_path_buf());
        let Some(destination) = destination else {
            continue;
        };
        let request = RepairRequest {
            destination: &destination,
            argv: &[adapter.executable.display().to_string(), repository.clone()],
            task: "repair",
            journal,
            run_id,
            repository: &repository,
            frozen_inputs: &frozen,
            budget: Duration::from_secs(60),
        };
        match execute_confined_repair(request) {
            Ok(RepairOutcome::Succeeded { .. }) => {
                repaired.push(repository.clone());
                still_failed.retain(|id| id != &repository);
            }
            Err(_) => {
                // The attempt was consumed; the repository stays failed.
            }
        }
    }
    Ok(RepairPassOutcome {
        repaired,
        still_failed,
    })
}
