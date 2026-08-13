//! Exact operation-scoped candidate tree.
//!
//! The candidate tree contains every and only the authorized delta entries:
//! each authorized path maps to the exact blob of the working-tree content
//! (hashed without filters), deleted paths are removals, and unrelated
//! staged/worktree content is excluded by construction.  A missing worktree
//! file for a non-deleted authorized change is drift and fails; ambiguous
//! file identities fail the boundary.

#![allow(dead_code)]

use super::state::{AuthorizedDelta, TargetChange};
use std::{error::Error, fmt, path::Path, process::Command};

/// One candidate tree entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TreeEntry {
    /// The path must carry the given blob (added/modified/type/mode/link).
    Blob { path: String, blob: String },
    /// The path must be removed.
    Removal { path: String },
}

/// The complete candidate tree for the operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationTree {
    pub entries: Vec<TreeEntry>,
}

/// Typed tree-construction failures.
#[derive(Debug)]
pub enum TreeError {
    Git {
        command: String,
        reason: String,
    },
    /// An authorized non-deleted change has no working-tree file.
    Drift {
        path: String,
    },
    UnsafePath {
        path: String,
    },
    Io {
        path: String,
        reason: String,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git { command, reason } => {
                write!(
                    formatter,
                    "git tree preparation failed ({command}): {reason}"
                )
            }
            Self::Drift { path } => {
                write!(
                    formatter,
                    "authorized change has no working-tree file: {path}"
                )
            }
            Self::UnsafePath { path } => {
                write!(formatter, "tree preparation rejected unsafe path {path:?}")
            }
            Self::Io { path, reason } => {
                write!(
                    formatter,
                    "tree preparation io failure for {path}: {reason}"
                )
            }
        }
    }
}
impl Error for TreeError {}

/// Build the exact candidate tree for the authorized delta.
pub fn build_operation_tree(
    root: &Path,
    delta: &AuthorizedDelta,
) -> Result<OperationTree, TreeError> {
    let mut entries = Vec::with_capacity(delta.changes().len());
    for change in delta.changes() {
        let path = String::from_utf8_lossy(change.target().path().as_bytes()).into_owned();
        validate_path(&path)?;
        match change.change() {
            TargetChange::Deleted => {
                entries.push(TreeEntry::Removal { path });
            }
            TargetChange::Renamed => {
                let from = String::from_utf8_lossy(
                    change.rename_from().expect("rename source").as_bytes(),
                )
                .into_owned();
                validate_path(&from)?;
                entries.push(TreeEntry::Removal { path: from });
                entries.push(TreeEntry::Blob {
                    blob: hash_worktree_file(root, &path)?,
                    path,
                });
            }
            TargetChange::Added
            | TargetChange::Modified
            | TargetChange::TypeChanged
            | TargetChange::ModeChanged
            | TargetChange::LinkChanged => {
                entries.push(TreeEntry::Blob {
                    blob: hash_worktree_file(root, &path)?,
                    path,
                });
            }
            TargetChange::Untracked => {
                return Err(TreeError::UnsafePath { path });
            }
        }
    }
    entries.sort_by(|a, b| entry_path(a).cmp(entry_path(b)));
    Ok(OperationTree { entries })
}

fn entry_path(entry: &TreeEntry) -> &str {
    match entry {
        TreeEntry::Blob { path, .. } | TreeEntry::Removal { path } => path,
    }
}

fn validate_path(path: &str) -> Result<(), TreeError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|component| component == "..")
    {
        return Err(TreeError::UnsafePath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

/// Hash the exact working-tree bytes without filters; a missing file for a
/// non-deleted authorized change is drift.
fn hash_worktree_file(root: &Path, path: &str) -> Result<String, TreeError> {
    let file = root.join(path);
    if !file.is_file() {
        return Err(TreeError::Drift {
            path: path.to_owned(),
        });
    }
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["hash-object", "--", path])
        .output()
        .map_err(|error| TreeError::Io {
            path: path.to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(TreeError::Git {
            command: "hash-object".to_owned(),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
