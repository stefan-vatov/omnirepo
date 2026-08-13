//! Push ambiguity reconciliation.
//!
//! After a bounded push attempt the exact remote ref is re-read: when the
//! remote already carries the recorded OID the push succeeded (even if the
//! previous attempt disconnected after accept) and nothing is repushed;
//! when the remote still carries the pre-push OID a retry is allowed
//! within policy; any third OID is a conflict and fails without force.
//! Crash/restart can never publish a different commit or ref: the refspec
//! is always the recorded OID against the frozen target.

#![allow(dead_code)]

use super::remote_target::FrozenRemoteTarget;
use crate::repository::RevisionId;
use std::{error::Error, fmt, path::Path, process::Command};

/// Reconciliation outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconcileOutcome {
    /// The remote carries the recorded OID: success, nothing to repush.
    Accepted { recorded: RevisionId },
    /// The remote still carries the pre-push OID: a retry is allowed within
    /// policy.
    RetryAllowed {
        recorded: RevisionId,
        remote: RevisionId,
    },
    /// The remote carries a third OID: a conflict; without force nothing is
    /// published.
    Conflict {
        recorded: RevisionId,
        remote: RevisionId,
    },
}

/// Reconciliation failures.
#[derive(Debug)]
pub enum ReconcileError {
    Git { command: String, reason: String },
    Transport { reason: String },
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git { command, reason } => {
                write!(
                    formatter,
                    "push reconciliation failed ({command}): {reason}"
                )
            }
            Self::Transport { reason } => {
                write!(formatter, "push reconciliation transport failure: {reason}")
            }
        }
    }
}
impl Error for ReconcileError {}

/// Reconcile the push of the recorded OID against the exact remote ref.
///
/// The remote ref is read with a bounded, noninteractive, hardened ls-remote
/// of exactly the selected reference; the recorded and pre-push OIDs come
/// from the caller's frozen state.
pub fn reconcile_push(
    working: &crate::platform::AuthorityRoot<
        crate::platform::GitWorkingDirectoryRoot,
        crate::platform::ReadOnly,
    >,
    target: &FrozenRemoteTarget,
    recorded: &RevisionId,
    pre_push: &RevisionId,
) -> Result<ReconcileOutcome, ReconcileError> {
    let remote_text = git_text(
        working.display_path().as_path(),
        &["ls-remote", &target.remote, target.reference.as_str()],
    )
    .map_err(|reason| ReconcileError::Git {
        command: format!("ls-remote {}", target.reference.as_str()),
        reason,
    })?;
    let remote_oid = remote_text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim();
    if remote_oid.is_empty() {
        // The selected ref does not exist remotely.
        return Ok(ReconcileOutcome::RetryAllowed {
            recorded: recorded.clone(),
            remote: pre_push.clone(),
        });
    }
    let remote = RevisionId::new(remote_oid).map_err(|error| ReconcileError::Transport {
        reason: format!("remote returned an invalid OID: {error}"),
    })?;
    if remote == *recorded {
        return Ok(ReconcileOutcome::Accepted {
            recorded: recorded.clone(),
        });
    }
    if remote == *pre_push {
        return Ok(ReconcileOutcome::RetryAllowed {
            recorded: recorded.clone(),
            remote,
        });
    }
    Ok(ReconcileOutcome::Conflict {
        recorded: recorded.clone(),
        remote,
    })
}

/// The hardened git text command (bounded, inert hooks/config, no optional
/// locks), identical to the repository-domain pattern.
pub(crate) fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .arg("--no-optional-locks")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("filter.lfs.smudge=")
        .arg("-c")
        .arg("filter.lfs.clean=")
        .arg("-c")
        .arg("filter.lfs.process=");
    let output = command
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

#[cfg(test)]
mod push_reconcile_tests;
