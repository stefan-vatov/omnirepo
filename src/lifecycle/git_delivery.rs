//! Scoped Git delivery after a verified pass.
//!
//! Only a Ready verification verdict may reach Git: the scoped authorized
//! delta is committed through the typed root with a journaled intent and
//! result, and the exact OID is reconciled against the object database.  A
//! non-ready verdict refuses before any Git contact.

#![allow(dead_code)]

use crate::lifecycle::commit_journal::{create_commit_journaled, reconcile_commit};

#[cfg(test)]
mod git_delivery_tests;
use crate::lifecycle::journal::JournalHandle;
use crate::lifecycle::verify_and_gate::VerificationVerdict;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::IsolatedIndex;
use std::{error::Error, fmt};

/// The delivery outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    /// The scoped delta was committed and the exact OID reconciles.
    Delivered { oid: String },
    /// Delivery was refused before any Git contact.
    Rejected { reason: String },
}

/// Delivery failures.
#[derive(Debug)]
pub enum DeliveryError {
    Commit { reason: String },
    Reconcile { reason: String },
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Commit { reason } => write!(formatter, "git delivery commit failure: {reason}"),
            Self::Reconcile { reason } => {
                write!(formatter, "git delivery reconcile failure: {reason}")
            }
        }
    }
}
impl Error for DeliveryError {}

/// Coordinate the scoped delivery.  The commit carries the exact scoped
/// delta; after creation the OID is reconciled against the object
/// database, so a crash cannot publish a different commit.
#[allow(clippy::too_many_arguments)]
pub fn coordinate_git_delivery(
    git_root: &AuthorityRoot<GitWorkingDirectoryRoot, ReadOnly>,
    index: &IsolatedIndex,
    base: Option<&str>,
    message: &str,
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    verdict: VerificationVerdict,
) -> Result<DeliveryOutcome, DeliveryError> {
    if verdict != VerificationVerdict::Ready {
        return Ok(DeliveryOutcome::Rejected {
            reason: "the verification verdict is not ready".to_owned(),
        });
    }
    let recorded =
        create_commit_journaled(git_root, index, base, message, journal, run_id, repository)
            .map_err(|error| DeliveryError::Commit {
                reason: error.to_string(),
            })?;
    let exists =
        reconcile_commit(git_root, &recorded.sha).map_err(|error| DeliveryError::Reconcile {
            reason: error.to_string(),
        })?;
    if !exists {
        return Err(DeliveryError::Reconcile {
            reason: "the recorded OID does not exist in the object database".to_owned(),
        });
    }
    Ok(DeliveryOutcome::Delivered { oid: recorded.sha })
}
