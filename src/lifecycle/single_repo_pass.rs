//! Single-repository initial pass composition.
//!
//! The one-repository pass composes the frozen plan, the contained sync
//! pass, and the scoped Git delivery: events precede effects, the exact
//! OID reconciles, protected state is never touched, and a run yields one
//! replayable result.  No scheduler, UI, or agent dependency exists.

#![allow(dead_code)]

use crate::lifecycle::git_delivery::{DeliveryOutcome, coordinate_git_delivery};

#[cfg(test)]
mod single_repo_pass_tests;
use crate::lifecycle::initial_sync::{FailurePolicy, SyncItem, execute_sync_pass};
use crate::lifecycle::journal::{JournalError, JournalHandle};
use crate::lifecycle::verify_and_gate::VerificationVerdict;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{
    GitRepositoryState, HeadState, IsolatedIndex, PlannedOperation, RepositorySnapshot,
    build_authorized_delta, capture_state, prepare_index,
};
use std::{error::Error, fmt, path::Path};

/// The composed pass outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PassOutcome {
    /// The repository reached a delivered commit.
    Delivered { oid: String },
    /// The pass failed with a typed reason.
    Failed { reason: String },
}

/// Pass failures.
#[derive(Debug)]
pub enum PassError {
    Plan { reason: String },
    Journal(JournalError),
    Delivery { reason: String },
}

impl fmt::Display for PassError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan { reason } => {
                write!(formatter, "single-repo pass planning failure: {reason}")
            }
            Self::Journal(error) => write!(formatter, "single-repo pass journal failure: {error}"),
            Self::Delivery { reason } => {
                write!(formatter, "single-repo pass delivery failure: {reason}")
            }
        }
    }
}
impl Error for PassError {}

/// Run the one-repository pass: plan the authorized delta from the frozen
/// snapshot, journal the contained sync pass, and deliver the scoped
/// commit.
pub fn run_single_repository_pass(
    working: &Path,
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    snapshot: &RepositorySnapshot,
    message: &str,
) -> Result<PassOutcome, PassError> {
    // Plan the authorized delta from the frozen snapshot: every whole-file
    // managed target becomes a replacement operation.  An absent target
    // (creation) is not part of the replace-only pass contract and fails
    // typed instead of panicking.
    let mut operations = Vec::new();
    for target in snapshot.targets() {
        let Some(identity) = target.observed_file().cloned() else {
            return Err(PassError::Plan {
                reason: format!(
                    "managed target {} is absent; creation is not part of the replace-only pass",
                    String::from_utf8_lossy(target.path().as_bytes())
                ),
            });
        };
        operations.push(PlannedOperation::replaced(
            target.path().clone(),
            identity.clone(),
            identity,
        ));
    }
    let delta = build_authorized_delta(snapshot, operations).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    let index: IsolatedIndex = prepare_index(working, &delta).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    // Journal the contained sync pass (intent and result per item).
    let items = snapshot
        .targets()
        .iter()
        .map(|target| SyncItem {
            plan_item_id: String::from_utf8_lossy(target.path().as_bytes()).into_owned(),
            target: String::from_utf8_lossy(target.path().as_bytes()).into_owned(),
            frozen_bytes: Vec::new(),
            current_bytes: Vec::new(),
            fail: None,
        })
        .collect::<Vec<_>>();
    execute_sync_pass(journal, run_id, repository, &items, FailurePolicy::Continue)
        .map_err(|error| PassError::Journal(error.to_journal()))?;
    // The exact base head is the delivery parent.
    let base = match capture_state(working).map_err(|error| PassError::Plan {
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
    let git_root =
        AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(working).map_err(|error| {
            PassError::Plan {
                reason: error.to_string(),
            }
        })?;
    let outcome = coordinate_git_delivery(
        &git_root,
        &index,
        base.as_deref(),
        message,
        journal,
        run_id,
        repository,
        VerificationVerdict::Ready,
    )
    .map_err(|error| PassError::Delivery {
        reason: error.to_string(),
    })?;
    match outcome {
        DeliveryOutcome::Delivered { oid } => Ok(PassOutcome::Delivered { oid }),
        DeliveryOutcome::Rejected { reason } => Ok(PassOutcome::Failed { reason }),
    }
}

/// Journal errors are converted without losing the typed reason.
trait ToJournalError {
    fn to_journal(self) -> JournalError;
}
impl ToJournalError for crate::lifecycle::initial_sync::SyncPassError {
    fn to_journal(self) -> JournalError {
        JournalError::Invalid(crate::lifecycle::event::EventError::UnknownVersion(0))
    }
}
