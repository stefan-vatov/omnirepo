//! Configured priority fallback within the durable repair budget.
//!
//! The eligible failed repositories are allocated repair attempts within
//! the durable budget: the configured priority list ranks first, then the
//! remaining repositories follow in input order.  Every allocation is
//! journaled as durable evidence before any repair executes, so a crash
//! after allocation never overspends the budget.

#![allow(dead_code)]

#[cfg(test)]
mod repair_fallback_tests;

use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent};
use crate::lifecycle::journal::{JournalError, JournalHandle};
use crate::lifecycle::repair_selection::EligibleRepair;
use std::{error::Error, fmt};

/// One budgeted repair allocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairAllocation {
    pub repository: String,
    pub attempts: u32,
}

/// Allocation failures.
#[derive(Debug)]
pub enum FallbackError {
    Journal(JournalError),
}

impl fmt::Display for FallbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(formatter, "repair fallback journal failure: {error}"),
        }
    }
}
impl Error for FallbackError {}

/// Allocate repair attempts within the durable budget.
///
/// The configured priority list ranks first (unknown entries are
/// ignored); the remaining eligible repositories follow in input order.
/// The budget is consumed attempt-by-attempt; once it is exhausted no
/// further allocation is made.  Pure: no I/O, no state.
pub fn allocate_within_budget(
    eligible: &[EligibleRepair],
    priority: &[String],
    budget: u32,
) -> Vec<RepairAllocation> {
    let mut remaining = budget;
    let mut allocations = Vec::new();
    let mut push = |repository: &str, attempts: u8, remaining: &mut u32| {
        if *remaining == 0 {
            return;
        }
        let attempts = u32::from(attempts).min(*remaining);
        *remaining -= attempts;
        allocations.push(RepairAllocation {
            repository: repository.to_owned(),
            attempts,
        });
    };
    // Configured priority first.
    for name in priority {
        if let Some(entry) = eligible.iter().find(|entry| &entry.repository == name) {
            push(&entry.repository, entry.attempts, &mut remaining);
        }
    }
    // Then the remaining repositories in input order.
    for entry in eligible {
        if priority.contains(&entry.repository) {
            continue;
        }
        push(&entry.repository, entry.attempts, &mut remaining);
    }
    allocations
}

/// Journal the allocations as durable evidence before any repair
/// executes.
pub fn commit_repair_allocations(
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    allocations: &[RepairAllocation],
    stage: &'static str,
) -> Result<(), FallbackError> {
    let payload = allocations
        .iter()
        .map(|allocation| format!("{}:{}", allocation.repository, allocation.attempts))
        .collect::<Vec<_>>()
        .join(",");
    let evidence = EvidenceRef::new(
        EvidenceKind::Process,
        format!("repair/{repository}/allocations/{stage}/{payload}"),
        1,
    )
    .map_err(|error| FallbackError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: Some(repository.to_owned()),
            evidence,
            stage: Some(stage),
        })
        .map_err(FallbackError::Journal)?;
    Ok(())
}
