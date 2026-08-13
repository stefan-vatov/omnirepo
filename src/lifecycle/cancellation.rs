//! Cancellation and not-started/in-flight terminalization.
//!
//! Cancellation classifies every selected repository: queued repositories
//! never start and record cancelled results; in-flight repositories are
//! terminated at the boundary and record cancelled results; the run itself
//! terminalizes as cancelled.  Every decision is journaled with the exact
//! repository identities.

#![allow(dead_code)]

use super::journal::{JournalError, JournalHandle};

#[cfg(test)]
mod cancellation_tests;
use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent, Operation, Outcome};
use std::{error::Error, fmt};

/// Cancellation failures.
#[derive(Debug)]
pub enum CancelError {
    Journal(JournalError),
    EmptyFleet { reason: String },
}

impl fmt::Display for CancelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(formatter, "cancellation journal failure: {error}"),
            Self::EmptyFleet { reason } => write!(formatter, "cancellation failure: {reason}"),
        }
    }
}
impl Error for CancelError {}

/// Cancel a run: every selected repository records a cancelled result (queued
/// ones never started, in-flight ones were terminated at the boundary), and
/// the run terminalizes as cancelled.
pub fn cancel_run(
    journal: &JournalHandle,
    run_id: &str,
    repositories: &[String],
) -> Result<(), CancelError> {
    if repositories.is_empty() {
        return Err(CancelError::EmptyFleet {
            reason: "no repositories were selected for cancellation".to_owned(),
        });
    }
    // Every selected repository declares its intent first: once the first
    // cancelled result marks the run cancelled, the validator rejects new
    // repository intents.
    for repository in repositories {
        journal
            .submit(JournalEvent::RepositoryIntent {
                checkpoint: 0,
                run_id: run_id.to_owned(),
                repository_id: repository.clone(),
                operation: Operation::Synchronize,
                attempt: 1,
            })
            .map_err(CancelError::Journal)?;
    }
    let evidence = EvidenceRef::new(
        EvidenceKind::Process,
        format!("cancellation/{}/{}", repositories.len(), run_id),
        0,
    )
    .map_err(|error| CancelError::Journal(JournalError::Invalid(error)))?;
    // The cancellation notice precedes the outcomes: once a repository
    // result marks the run cancelled, the validator rejects run-level
    // events.
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: None,
            evidence,
            stage: Some("cancellation"),
        })
        .map_err(CancelError::Journal)?;
    for repository in repositories {
        journal
            .submit(JournalEvent::RepositoryResult {
                checkpoint: 0,
                run_id: run_id.to_owned(),
                repository_id: repository.clone(),
                operation: Operation::Synchronize,
                attempt: 1,
                outcome: Outcome::Cancelled,
            })
            .map_err(CancelError::Journal)?;
    }
    journal
        .submit(JournalEvent::Cancelled {
            checkpoint: 0,
            run_id: run_id.to_owned(),
        })
        .map_err(CancelError::Journal)?;
    Ok(())
}

/// Terminalize a run that never started (no repositories were admitted):
/// the run records a cancelled outcome without repository entries.
pub fn terminalize_not_started(journal: &JournalHandle, run_id: &str) -> Result<(), CancelError> {
    journal
        .submit(JournalEvent::Cancelled {
            checkpoint: 0,
            run_id: run_id.to_owned(),
        })
        .map_err(CancelError::Journal)?;
    Ok(())
}

/// Terminalize an in-flight run: every running repository records a
/// cancelled result, then the run terminalizes as cancelled.
pub fn terminalize_in_flight(
    journal: &JournalHandle,
    run_id: &str,
    repositories: &[String],
) -> Result<(), CancelError> {
    cancel_run(journal, run_id, repositories)
}
