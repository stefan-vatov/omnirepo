//! Pre-attempt repository snapshot and frozen-input verification.
//!
//! Before a confined repair attempt the repository state is snapshotted:
//! the exact head OID, the managed content identity, and the frozen
//! baseline and lineage identities.  The frozen repair inputs are then
//! verified against the snapshot — a mismatch aborts the attempt.

#![allow(dead_code)]

use crate::repository::capture_state;

#[cfg(test)]
mod repair_snapshot_tests;
use crate::platform::RelativePath;
use crate::repository::HeadState;
use std::{error::Error, fmt, path::Path};

/// The pre-attempt snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreAttemptSnapshot {
    pub repository: String,
    pub head_oid: String,
    pub managed_identity: String,
    pub baseline_identity: String,
    pub frozen_lineage_identity: String,
}

/// Snapshot failures.
#[derive(Debug)]
pub enum SnapshotError {
    Root { reason: String },
    Capture { reason: String },
    ManagedPath { reason: String },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { reason } => write!(formatter, "repair snapshot root failure: {reason}"),
            Self::Capture { reason } => {
                write!(formatter, "repair snapshot capture failure: {reason}")
            }
            Self::ManagedPath { reason } => {
                write!(formatter, "repair snapshot managed path failure: {reason}")
            }
        }
    }
}
impl Error for SnapshotError {}

/// Snapshot the pre-attempt state: the exact head OID and the managed
/// content identity, alongside the frozen baseline and lineage identities.
pub fn snapshot_pre_attempt(
    working: &Path,
    baseline_identity: &str,
    frozen_lineage_identity: &str,
) -> Result<PreAttemptSnapshot, SnapshotError> {
    if !working.is_dir() {
        return Err(SnapshotError::Root {
            reason: "the working directory is not a directory".to_owned(),
        });
    }
    let captured = capture_state(working).map_err(|error| SnapshotError::Capture {
        reason: error.to_string(),
    })?;
    let head_oid = match captured {
        crate::repository::GitRepositoryState::Git(facts) => match facts.head() {
            HeadState::Attached { commit, .. } | HeadState::Detached { commit } => {
                commit.as_str().to_owned()
            }
            HeadState::Unborn => String::new(),
        },
        crate::repository::GitRepositoryState::NonGit => String::new(),
    };
    // The managed content identity: the canonical representation of the
    // managed target path (whole-file identity witness).
    let managed =
        RelativePath::parse("managed.txt").map_err(|error| SnapshotError::ManagedPath {
            reason: error.to_string(),
        })?;
    let managed_identity = format!(
        "managed:{}:{}",
        managed.display(),
        fs_identity_witness(working)
    );
    Ok(PreAttemptSnapshot {
        repository: working.display().to_string(),
        head_oid,
        managed_identity,
        baseline_identity: baseline_identity.to_owned(),
        frozen_lineage_identity: frozen_lineage_identity.to_owned(),
    })
}

/// Verify the frozen repair inputs against the snapshot: the baseline and
/// lineage identities must match exactly.
pub fn verify_frozen_inputs(snapshot: &PreAttemptSnapshot, frozen: &[String]) -> bool {
    let mut matched = 0;
    for input in frozen {
        if input == &snapshot.baseline_identity || input == &snapshot.frozen_lineage_identity {
            matched += 1;
        }
    }
    matched == frozen.len() && !frozen.is_empty()
}

fn fs_identity_witness(working: &Path) -> String {
    let managed = working.join("managed.txt");
    match std::fs::metadata(&managed) {
        Ok(metadata) => format!(
            "{}:{}",
            metadata.len(),
            metadata
                .modified()
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                })
                .unwrap_or(0)
        ),
        Err(_) => "absent".to_owned(),
    }
}
