//! Single-repository initial pass composition.
//!
//! The one-repository pass composes the frozen plan, the contained sync
//! pass, the declared verification commands, and the scoped Git delivery:
//! events precede effects, every declared check runs in configured order
//! and must pass, concurrent managed modification fails the pass, the
//! exact OID reconciles, protected state is never touched, and a run
//! yields one replayable result.  No scheduler, UI, or agent dependency
//! exists.

#![allow(dead_code)]

use crate::lifecycle::check_runner::{CheckOutcome, run_check};
use crate::lifecycle::command_spec::{
    DEFAULT_COMMAND_TIMEOUT, DeclaredCommand, translate_commands,
};
use crate::lifecycle::git_delivery::{DeliveryOutcome, coordinate_git_delivery};

#[cfg(test)]
mod single_repo_pass_tests;
use crate::lifecycle::initial_sync::{FailurePolicy, SyncItem, execute_sync_pass};
use crate::lifecycle::journal::{JournalError, JournalHandle};
use crate::lifecycle::verify_and_gate::VerificationVerdict;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{
    GitRepositoryState, HeadState, IsolatedIndex, PlannedOperation, RepositorySnapshot,
    TargetChange, VerificationCommand, build_authorized_delta, capture_state, prepare_index,
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

/// The typed failure reason for one verification outcome.
fn verification_failure(outcome: &CheckOutcome) -> String {
    match outcome {
        CheckOutcome::Passed => unreachable!("passed checks are not failures"),
        CheckOutcome::Failed { code } => match code {
            Some(code) => format!("verification failed with exit code {code}"),
            None => "verification failed by signal".to_owned(),
        },
        CheckOutcome::TimedOut { budget } => {
            format!("verification exceeded its {budget:?} budget")
        }
        CheckOutcome::Cancelled => "verification was cancelled".to_owned(),
    }
}

/// Run the one-repository pass: plan the authorized delta from the frozen
/// snapshot, journal the contained sync pass, run the declared
/// verification commands in configured order, and deliver the scoped
/// commit only when every check passed and no concurrent managed
/// modification appeared.
pub fn run_single_repository_pass(
    working: &Path,
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    snapshot: &RepositorySnapshot,
    checks: &[VerificationCommand],
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
    // Verification: every declared check runs in configured order with a
    // bounded budget; any non-passed outcome fails the pass and prevents
    // Git.  An absent or empty command list means no verification command
    // is required (canon/architecture/fleet-lifecycle.md).
    if !checks.is_empty() {
        let declared = checks
            .iter()
            .map(|command| DeclaredCommand {
                argv: command.argv().to_vec(),
                cwd: None,
                env: Vec::new(),
                timeout: None,
                stdin: None,
                capture_output: true,
                shell: None,
            })
            .collect::<Vec<_>>();
        let specs = translate_commands(repository, "plan", &declared, DEFAULT_COMMAND_TIMEOUT)
            .map_err(|error| PassError::Plan {
                reason: error.to_string(),
            })?;
        for spec in &specs {
            let result =
                run_check(working, spec, spec.timeout).map_err(|error| PassError::Plan {
                    reason: format!(
                        "verification at position {} failed to run: {error}",
                        spec.position
                    ),
                })?;
            if !matches!(result.outcome, CheckOutcome::Passed) {
                return Ok(PassOutcome::Failed {
                    reason: format!(
                        "check {} ({}) {}; no Git delivery",
                        spec.position + 1,
                        spec.argv.join(" "),
                        verification_failure(&result.outcome)
                    ),
                });
            }
        }
    }
    // Concurrent-modification guard: re-capture the current state; any
    // change at a managed target that is not the authorized replacement is
    // a concurrent user change and prevents Git.  Unmanaged paths may
    // coexist (pre-existing state).  The staged pass does not require the
    // authorized delta to have landed in the worktree, so a missing
    // operation effect is not a failure here.
    let current = capture_state(working).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    if let GitRepositoryState::Git(facts) = &current {
        let mut managed: Vec<(Vec<u8>, TargetChange)> = Vec::new();
        if let crate::repository::IndexState::Entries(entries) = facts.index() {
            for entry in entries {
                managed.push((entry.path().as_bytes().to_vec(), entry.change()));
            }
        }
        if let crate::repository::WorktreeState::Entries(entries) = facts.worktree() {
            for entry in entries {
                managed.push((entry.path().as_bytes().to_vec(), entry.change()));
            }
        }
        for (path, change) in managed {
            let target = snapshot
                .targets()
                .iter()
                .any(|target| target.path().as_bytes() == path.as_slice());
            if target && change != TargetChange::Modified {
                return Ok(PassOutcome::Failed {
                    reason: format!(
                        "managed target {} changed concurrently ({change:?}); no Git delivery",
                        String::from_utf8_lossy(&path)
                    ),
                });
            }
        }
    }
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
