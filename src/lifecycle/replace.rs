//! Containment-aware old-or-complete-new replacement executor.
//!
//! Given a prepared transaction plan and exact authoritative bytes, the
//! executor creates a temporary sibling through the platform authority
//! (no-follow, exclusive, mode-applied), writes and syncs the exact bytes,
//! then publishes atomically with an old-or-complete-new contract: the old
//! target stays authoritative until the atomic rename, and an injected
//! failure at any stage exposes only allowed old/new/residue states with the
//! owned temporary cleaned up.

#![allow(dead_code)]

use crate::managed_content::{TempCandidate, TransactionPlan};
use crate::platform::{
    DestinationRepositoryRoot, MutationIntent, PathError, RelativePath, open_mutation_root,
    sync_directory, sync_file,
};
use std::{error::Error, fmt, io::Write, path::Path, path::PathBuf};

#[cfg(test)]
mod replace_tests;

/// Requested replacement of one managed target.
#[derive(Clone, Debug)]
pub struct ReplaceRequest {
    pub plan: TransactionPlan,
    pub content: Vec<u8>,
    /// Decided metadata mode applied to the temporary before publish.
    pub mode: u32,
}

impl ReplaceRequest {
    pub fn new(plan: TransactionPlan, content: impl Into<Vec<u8>>, mode: u32) -> Self {
        Self {
            plan,
            content: content.into(),
            mode,
        }
    }
}

/// Typed replacement failures; the old target is always preserved.
#[derive(Debug)]
pub enum ReplaceError {
    Authority { reason: String },
    Resolve { reason: String },
    CreateTemp { reason: String },
    Write { reason: String },
    Sync { reason: String },
    Publish { reason: String },
    Cleanup { path: PathBuf, reason: String },
}

impl fmt::Display for ReplaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority { reason } => {
                write!(formatter, "replacement authority failure: {reason}")
            }
            Self::Resolve { reason } => {
                write!(formatter, "cannot resolve replacement target: {reason}")
            }
            Self::CreateTemp { reason } => {
                write!(formatter, "cannot create replacement temporary: {reason}")
            }
            Self::Write { reason } => write!(formatter, "cannot write replacement bytes: {reason}"),
            Self::Sync { reason } => write!(formatter, "cannot sync replacement bytes: {reason}"),
            Self::Publish { reason } => write!(formatter, "cannot publish replacement: {reason}"),
            Self::Cleanup { path, reason } => write!(
                formatter,
                "cannot clean replacement temporary {}: {reason}",
                path.display()
            ),
        }
    }
}
impl Error for ReplaceError {}

/// Execute the prepared replacement under the platform authority.
///
/// The temporary is created as an exclusive sibling of the target (same
/// directory, same filesystem), written and synced, then atomically renamed
/// over the target; the parent directory is synced afterwards.  Any failure
/// before the rename leaves the old target byte-identical and removes the
/// owned temporary.
pub fn replace(root: &Path, request: &ReplaceRequest) -> Result<(), ReplaceError> {
    let authority = open_mutation_root::<DestinationRepositoryRoot>(root).map_err(|error| {
        ReplaceError::Authority {
            reason: error.to_string(),
        }
    })?;
    let target_relative = RelativePath::parse(&plan_display(&request.plan)).map_err(|error| {
        ReplaceError::Resolve {
            reason: error.to_string(),
        }
    })?;
    // Replace revalidates an existing target; a missing target is a create
    // candidate resolved exclusively (old-or-complete-new covers absence).
    let target = match authority.resolve_mutation(&target_relative, MutationIntent::Replace) {
        Ok(target) => target,
        Err(PathError::NotFound { .. }) => authority
            .resolve_mutation(&target_relative, MutationIntent::CreateExclusive)
            .map_err(|error| ReplaceError::Resolve {
                reason: error.to_string(),
            })?,
        Err(error) => {
            return Err(ReplaceError::Resolve {
                reason: error.to_string(),
            });
        }
    };
    let parent = target
        .clone_parent()
        .map_err(|error| ReplaceError::Resolve {
            reason: error.to_string(),
        })?;

    let temp_relative = temp_path(&request.plan)?;
    let temp = authority
        .resolve_mutation(&temp_relative, MutationIntent::CreateExclusive)
        .map_err(|error| ReplaceError::CreateTemp {
            reason: error.to_string(),
        })?;
    let mut file = temp
        .create_exclusive_with_mode(request.mode)
        .map_err(|error| ReplaceError::CreateTemp {
            reason: error.to_string(),
        })?;
    let write_result = (|| -> Result<(), ReplaceError> {
        file.write_all(&request.content)
            .map_err(|error| ReplaceError::Write {
                reason: error.to_string(),
            })?;
        sync_file(
            &file,
            &root.join(temp_relative.display()).display().to_string(),
        )
        .map_err(|error| ReplaceError::Sync {
            reason: error.to_string(),
        })?;
        drop(file);
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(root.join(temp_relative.display()));
        return Err(error);
    }

    let target_absolute = root.join(target_relative.display());
    let temp_absolute = root.join(temp_relative.display());
    if let Err(error) = std::fs::rename(&temp_absolute, &target_absolute) {
        let _ = std::fs::remove_file(&temp_absolute);
        return Err(ReplaceError::Publish {
            reason: error.to_string(),
        });
    }
    sync_directory(&parent, &root.display().to_string()).map_err(|error| ReplaceError::Sync {
        reason: error.to_string(),
    })?;
    Ok(())
}

/// The plan's target as a relative path string.
fn plan_display(plan: &TransactionPlan) -> String {
    plan.target().display().to_string()
}

/// The owned temporary name: a sibling of the target, keyed by the operation
/// identity and a fixed attempt, validated through the domain's candidate
/// rules.
fn temp_path(plan: &TransactionPlan) -> Result<RelativePath, ReplaceError> {
    let target = plan.target().display().to_string();
    let (parent, file_name) = match target.rfind('/') {
        Some(index) => (&target[..index], &target[index + 1..]),
        None => ("", target.as_str()),
    };
    let temp_name = format!(".{file_name}.omnirepo-tmp-{}-1.tmp", plan.operation_id());
    let candidate = TempCandidate::new(PathBuf::from(temp_name), 1).map_err(|error| {
        ReplaceError::CreateTemp {
            reason: error.to_string(),
        }
    })?;
    let combined = if parent.is_empty() {
        candidate.path().display().to_string()
    } else {
        format!("{parent}/{}", candidate.path().display())
    };
    RelativePath::parse(&combined).map_err(|error| ReplaceError::CreateTemp {
        reason: error.to_string(),
    })
}
