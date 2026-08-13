//! Snapshot revalidation and delta classification at lifecycle boundaries.
//!
//! The planned managed-content operations become the authorized delta
//! against the frozen baseline.  The current captured state then classifies
//! per change: exactly the authorized change is the expected operation
//! effect; unmanaged changes are pre-existing state; managed changes outside
//! the delta are concurrent user changes; conflicts fail as ambiguous.  A
//! planned change that did not land is a forbidden missing effect.
//! Revalidation never runs hooks, helpers, or network.

#![allow(dead_code)]

use super::capture::capture_state;
use super::manifest::{PlannedOperation, build_authorized_delta};
use super::state::{
    GitRepositoryState, IndexState, RepositorySnapshot, TargetChange, WorktreeState,
};
use std::{error::Error, fmt, path::Path};

/// Classification of one current change against the frozen baseline/delta.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Classification {
    /// The change is exactly the operation's authorized change.
    ExpectedOperation,
    /// Unmanaged content changed; pre-existing state remains distinguishable.
    PreExisting,
    /// Managed content changed outside the authorized delta: a concurrent
    /// user change the operation may not own.
    ConcurrentUserChange,
    /// The current change conflicts with the authorized change for the same
    /// managed target.
    Ambiguous,
    /// An authorized change that did not land in the current state.
    MissingOperationEffect,
}

/// One classified path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassifiedPath {
    pub path: String,
    pub classification: Classification,
}

/// Revalidation outcome: the classified view plus a hard verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Revalidation {
    pub paths: Vec<ClassifiedPath>,
    pub has_concurrent_or_ambiguous: bool,
}

/// Typed revalidation failures; the boundary fails closed.
#[derive(Debug)]
pub enum RevalidateError {
    Capture(String),
    Delta(String),
}

impl fmt::Display for RevalidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capture(reason) => write!(formatter, "revalidation capture failed: {reason}"),
            Self::Delta(reason) => write!(formatter, "revalidation delta failed: {reason}"),
        }
    }
}
impl Error for RevalidateError {}

/// Revalidate a repository root against the frozen baseline and the planned
/// operations of the current lifecycle stage.
pub fn revalidate(
    root: &Path,
    baseline: &RepositorySnapshot,
    planned: Vec<PlannedOperation>,
) -> Result<Revalidation, RevalidateError> {
    let delta = build_authorized_delta(baseline, planned)
        .map_err(|error| RevalidateError::Delta(error.to_string()))?;
    let current =
        capture_state(root).map_err(|error| RevalidateError::Capture(error.to_string()))?;
    let GitRepositoryState::Git(facts) = &current else {
        return Ok(Revalidation {
            paths: Vec::new(),
            has_concurrent_or_ambiguous: false,
        });
    };

    let mut paths = Vec::new();
    let mut has_concurrent_or_ambiguous = false;
    let mut current_seen = std::collections::BTreeSet::new();
    for (path, change) in current_changes(facts) {
        current_seen.insert(path.clone());
        let is_managed = baseline
            .targets()
            .iter()
            .any(|target| target.path().as_bytes() == path.as_slice());
        let authorized = delta
            .changes()
            .iter()
            .find(|authorized| authorized.target().path().as_bytes() == path.as_slice());
        let classification = match (is_managed, authorized) {
            (_, Some(authorized)) => {
                if authorized.change() == change {
                    Classification::ExpectedOperation
                } else {
                    has_concurrent_or_ambiguous = true;
                    Classification::Ambiguous
                }
            }
            (true, None) => {
                has_concurrent_or_ambiguous = true;
                Classification::ConcurrentUserChange
            }
            (false, None) => Classification::PreExisting,
        };
        paths.push(ClassifiedPath {
            path: String::from_utf8_lossy(&path).into_owned(),
            classification,
        });
    }
    // Every authorized change must have landed; a missing effect fails.
    for authorized in delta.changes() {
        let path = authorized.target().path().as_bytes().to_vec();
        if !current_seen.contains(&path) {
            has_concurrent_or_ambiguous = true;
            paths.push(ClassifiedPath {
                path: String::from_utf8_lossy(&path).into_owned(),
                classification: Classification::MissingOperationEffect,
            });
        }
    }
    paths.sort_by(|a, b| a.path.cmp(&b.path));
    paths.dedup_by(|a, b| a.path == b.path && a.classification == b.classification);
    Ok(Revalidation {
        paths,
        has_concurrent_or_ambiguous,
    })
}

/// The current index/worktree changes as (path bytes, change).
fn current_changes(facts: &super::state::GitFacts) -> Vec<(Vec<u8>, TargetChange)> {
    let mut changes = Vec::new();
    if let IndexState::Entries(entries) = facts.index() {
        for entry in entries {
            changes.push((entry.path().as_bytes().to_vec(), entry.change()));
        }
    }
    if let WorktreeState::Entries(entries) = facts.worktree() {
        for entry in entries {
            changes.push((entry.path().as_bytes().to_vec(), entry.change()));
        }
    }
    changes
}
