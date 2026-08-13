//! Journaled operation commits and local ambiguity reconciliation.
//!
//! Before creating the commit, the commit intent is journaled; after
//! creation, the result is journaled with the exact OID and ref.  When the
//! journal lacks the result but the OID exists in the object database, the
//! ambiguity reconciles to the recorded commit (the result is re-journaled);
//! when the OID does not exist, the commit was never created and the
//! boundary fails.

#![allow(dead_code)]

use crate::repository::{CommitError, RecordedCommit, create_commit};

use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent, Operation, Outcome};
use crate::lifecycle::journal::{JournalError, JournalHandle};
use std::{error::Error, fmt, path::Path, process::Command};

#[cfg(test)]
mod commit_journal_tests;

#[cfg(test)]
mod commit_journal_fixture_tests;

/// Journaled commit failures.
#[derive(Debug)]
pub enum JournaledCommitError {
    Commit(CommitError),
    Journal(JournalError),
    /// The OID exists in the object database but no journal result exists
    /// and the repository ref is ambiguous.
    AmbiguousRef {
        oid: String,
    },
}

impl fmt::Display for JournaledCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Commit(error) => write!(formatter, "{error}"),
            Self::Journal(error) => write!(formatter, "commit journal failure: {error}"),
            Self::AmbiguousRef { oid } => {
                write!(formatter, "commit {oid} exists but its ref is ambiguous")
            }
        }
    }
}
impl Error for JournaledCommitError {}

/// Create and journal an operation commit with its exact OID and ref.
pub fn create_commit_journaled(
    root: &Path,
    index: &crate::repository::IsolatedIndex,
    parent: Option<&str>,
    message: &str,
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
) -> Result<RecordedCommit, JournaledCommitError> {
    // Intent before effect.
    journal
        .submit(JournalEvent::RepositoryIntent {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: repository.to_owned(),
            operation: Operation::Commit,
            attempt: 1,
        })
        .map_err(JournaledCommitError::Journal)?;
    let recorded =
        create_commit(root, index, parent, message).map_err(JournaledCommitError::Commit)?;
    // Result after effect with the exact OID.
    journal
        .submit(JournalEvent::RepositoryResult {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: repository.to_owned(),
            operation: Operation::Commit,
            attempt: 1,
            outcome: Outcome::Success,
        })
        .map_err(JournaledCommitError::Journal)?;
    let evidence = EvidenceRef::new(EvidenceKind::Git, format!("commit/{}", recorded.sha), 40)
        .map_err(|error| JournaledCommitError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: Some(repository.to_owned()),
            evidence,
            stage: Some("commit"),
        })
        .map_err(JournaledCommitError::Journal)?;
    Ok(recorded)
}

/// Reconcile local commit ambiguity: the journal result was lost but the
/// OID may exist in the object database.
pub fn reconcile_commit(root: &Path, oid: &str) -> Result<bool, JournaledCommitError> {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["cat-file", "-e", &format!("{oid}^{{commit}}")])
        .output()
        .map_err(|error| {
            JournaledCommitError::Commit(CommitError::Git {
                command: "cat-file".to_owned(),
                reason: error.to_string(),
            })
        })?;
    Ok(output.status.success())
}
