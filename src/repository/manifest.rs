//! Authorized-delta manifest construction from plan, baseline, and stage.
//!
//! The frozen plan's managed-content operations combine with the pre-effect
//! repository snapshot (frozen targets, witnesses, base HEAD, authority) into
//! the exact set of changes the current operation may own.  Every change
//! binds to the baseline's observed file identity; partial-file content
//! outside the managed boundary is excluded by the plan itself; empty and
//! changed deltas are explicit.

#![allow(dead_code)]

use super::state::{
    AuthorizedChange, AuthorizedDelta, DomainError, FileIdentity, ManagedTargetIdentity,
    RelativePath, RepositorySnapshot, TargetChange,
};

/// One planned managed-content operation for the current lifecycle stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedOperation {
    pub path: RelativePath,
    pub change: TargetChange,
    /// Identity observed before the operation (the baseline's frozen file).
    pub before: Option<FileIdentity>,
    /// Identity produced by the operation (the published file).
    pub after: Option<FileIdentity>,
}

impl PlannedOperation {
    pub fn added(path: RelativePath, after: FileIdentity) -> Self {
        Self {
            path,
            change: TargetChange::Added,
            before: None,
            after: Some(after),
        }
    }

    pub fn replaced(path: RelativePath, before: FileIdentity, after: FileIdentity) -> Self {
        Self {
            path,
            change: TargetChange::Modified,
            before: Some(before),
            after: Some(after),
        }
    }

    pub fn deleted(path: RelativePath, before: FileIdentity) -> Self {
        Self {
            path,
            change: TargetChange::Deleted,
            before: Some(before),
            after: None,
        }
    }
}

/// Manifest construction failures.
#[derive(Debug)]
pub enum ManifestError {
    /// The operation targets a path the baseline did not freeze.
    UnauthorizedTarget {
        path: String,
    },
    /// The operation's before identity does not match the baseline's frozen
    /// observed file — the baseline changed after it was captured.
    BaselineMismatch {
        path: String,
    },
    Domain(DomainError),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnauthorizedTarget { path } => {
                write!(formatter, "operation targets an unfrozen path: {path}")
            }
            Self::BaselineMismatch { path } => write!(
                formatter,
                "operation before identity does not match the frozen baseline: {path}"
            ),
            Self::Domain(error) => write!(formatter, "manifest domain failure: {error}"),
        }
    }
}
impl std::error::Error for ManifestError {}

/// Build the exact authorized delta for the current operation from the
/// frozen baseline snapshot and the planned managed-content operations.
pub fn build_authorized_delta(
    baseline: &RepositorySnapshot,
    operations: Vec<PlannedOperation>,
) -> Result<AuthorizedDelta, ManifestError> {
    let mut changes = Vec::with_capacity(operations.len());
    for operation in operations {
        let frozen = baseline.targets().iter().find(|target| {
            target.path() == &operation.path && matches!(operation.change, TargetChange::Added)
                || target.path() == &operation.path
        });
        let frozen = match frozen {
            Some(target) => target,
            None => {
                if operation.change == TargetChange::Added {
                    // An added target is created from absence; it must not be
                    // frozen in the baseline.
                    let target = ManagedTargetIdentity::whole_file(operation.path.clone(), None)
                        .map_err(ManifestError::Domain)?;
                    changes.push(
                        AuthorizedChange::new(
                            target,
                            TargetChange::Added,
                            None,
                            operation.after.clone(),
                        )
                        .map_err(ManifestError::Domain)?,
                    );
                    continue;
                }
                return Err(ManifestError::UnauthorizedTarget {
                    path: String::from_utf8_lossy(operation.path.as_bytes()).into_owned(),
                });
            }
        };
        // The operation's before identity must equal the frozen observed
        // identity: the baseline is the authority for pre-effect state.
        if frozen.observed_file() != operation.before.as_ref() {
            return Err(ManifestError::BaselineMismatch {
                path: String::from_utf8_lossy(operation.path.as_bytes()).into_owned(),
            });
        }
        let target = ManagedTargetIdentity::whole_file(operation.path, operation.before.clone())
            .map_err(ManifestError::Domain)?;
        changes.push(
            AuthorizedChange::new(target, operation.change, operation.before, operation.after)
                .map_err(ManifestError::Domain)?,
        );
    }
    AuthorizedDelta::from_snapshot(baseline, changes).map_err(ManifestError::Domain)
}
