//! Deliver valid repair changes through scoped Git and journal the
//! outcome.
//!
//! After the frozen verification passed, the verified repair change is
//! delivered as one scoped commit: the authorized delta is planned from
//! the frozen snapshot, staged into an isolated index, and committed
//! journaled — the intent precedes the effect and the exact OID is
//! reconciled against the object database before the outcome is
//! recorded.

#![allow(dead_code)]

#[cfg(test)]
mod repair_deliver_tests;

use crate::lifecycle::commit_journal::JournaledCommitError;
use crate::lifecycle::git_delivery::{DeliveryError, DeliveryOutcome, coordinate_git_delivery};
use crate::lifecycle::journal::JournalHandle;
use crate::lifecycle::verify_and_gate::VerificationVerdict;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{
    GitRepositoryState, HeadState, IsolatedIndex, PlannedOperation, RepositorySnapshot,
    build_authorized_delta, capture_state, prepare_index,
};
use std::{error::Error, fmt, path::Path};

/// The delivery outcome for one repair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairDelivery {
    /// The exact delivered commit OID.
    pub oid: String,
}

/// Delivery failures.
#[derive(Debug)]
pub enum RepairDeliveryError {
    Plan { reason: String },
    Delivery(DeliveryError),
    JournaledCommit(JournaledCommitError),
}

impl fmt::Display for RepairDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan { reason } => {
                write!(formatter, "repair delivery planning failure: {reason}")
            }
            Self::Delivery(error) => write!(formatter, "repair delivery failure: {error}"),
            Self::JournaledCommit(error) => {
                write!(
                    formatter,
                    "repair delivery journaled commit failure: {error}"
                )
            }
        }
    }
}
impl Error for RepairDeliveryError {}

/// Deliver the verified repair change as one scoped, journaled commit.
///
/// The frozen snapshot authorizes the whole-file replacement of every
/// managed target; the delta is staged in an isolated index; the exact
/// OID is reconciled before the outcome is reported.
pub fn deliver_repair_changes(
    working: &Path,
    snapshot: &RepositorySnapshot,
    message: &str,
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
) -> Result<RepairDelivery, RepairDeliveryError> {
    let operations = snapshot
        .targets()
        .iter()
        .map(|target| {
            PlannedOperation::replaced(
                target.path().clone(),
                target.observed_file().cloned().expect("frozen identity"),
                target.observed_file().cloned().expect("frozen identity"),
            )
        })
        .collect::<Vec<_>>();
    let delta = build_authorized_delta(snapshot, operations).map_err(|error| {
        RepairDeliveryError::Plan {
            reason: error.to_string(),
        }
    })?;
    let index: IsolatedIndex =
        prepare_index(working, &delta).map_err(|error| RepairDeliveryError::Plan {
            reason: error.to_string(),
        })?;
    let base = match capture_state(working).map_err(|error| RepairDeliveryError::Plan {
        reason: error.to_string(),
    })? {
        GitRepositoryState::Git(facts) => match facts.head() {
            HeadState::Attached { commit, .. } | HeadState::Detached { commit } => {
                Some(commit.as_str().to_owned())
            }
            HeadState::Unborn => None,
        },
        GitRepositoryState::NonGit => None,
    };
    let root =
        AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(working).map_err(|error| {
            RepairDeliveryError::Plan {
                reason: error.to_string(),
            }
        })?;
    let outcome = coordinate_git_delivery(
        &root,
        &index,
        base.as_deref(),
        message,
        journal,
        run_id,
        repository,
        VerificationVerdict::Ready,
    )
    .map_err(RepairDeliveryError::Delivery)?;
    match outcome {
        DeliveryOutcome::Delivered { oid } => Ok(RepairDelivery { oid }),
        DeliveryOutcome::Rejected { reason } => Err(RepairDeliveryError::Plan { reason }),
    }
}
