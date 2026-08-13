//! Owner-contracted operation commit creation.
//!
//! The isolated index becomes a tree object and then a commit object through
//! `write-tree` + `commit-tree` under the sanitized environment: no hooks
//! (commit-msg, pre-commit), no filters, no branch or worktree mutation, and
//! no config-driven widening.  The commit is recorded (its exact sha
//! returned) and the caller decides publication.

#![allow(dead_code)]

use super::git_index::IsolatedIndex;
use std::{error::Error, fmt, path::Path, process::Command};

/// The recorded operation commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedCommit {
    pub sha: String,
    pub tree: String,
    pub parent: Option<String>,
}

/// Typed commit-creation failures; nothing is widened on failure.
#[derive(Debug)]
pub enum CommitError {
    Git { command: String, reason: String },
    MissingTree { reason: String },
}

impl fmt::Display for CommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git { command, reason } => {
                write!(formatter, "operation commit failed ({command}): {reason}")
            }
            Self::MissingTree { reason } => {
                write!(formatter, "operation commit has no tree: {reason}")
            }
        }
    }
}
impl Error for CommitError {}

/// Create the owner-contracted commit from the isolated index without
/// touching the branch, worktree, or any hook.
pub fn create_commit(
    root: &Path,
    index: &IsolatedIndex,
    parent: Option<&str>,
    message: &str,
) -> Result<RecordedCommit, CommitError> {
    let tree = git_text_with_index(root, &["write-tree"], &index.index_path)?;
    let tree = tree.trim().to_owned();
    if tree.is_empty() {
        return Err(CommitError::MissingTree {
            reason: "write-tree produced no tree object".to_owned(),
        });
    }
    let mut arguments = vec!["commit-tree".to_owned(), tree.clone()];
    if let Some(parent) = parent {
        arguments.push("-p".to_owned());
        arguments.push(parent.to_owned());
    }
    arguments.push("-m".to_owned());
    arguments.push(message.to_owned());
    let sha = git_text_with_index(
        root,
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
        &index.index_path,
    )?;
    let sha = sha.trim().to_owned();
    if sha.is_empty() {
        return Err(CommitError::MissingTree {
            reason: "commit-tree produced no commit".to_owned(),
        });
    }
    Ok(RecordedCommit {
        sha,
        tree,
        parent: parent.map(str::to_owned),
    })
}

fn sanitized_command(root: &Path, index: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_INDEX_FILE", index);
    command.arg("--no-optional-locks");
    command.arg("-c").arg("core.hooksPath=/dev/null");
    command.arg("-c").arg("core.fsmonitor=false");
    command.arg("-c").arg("core.untrackedCache=false");
    command.arg("-c").arg("filter.lfs.smudge=");
    command.arg("-c").arg("filter.lfs.clean=");
    command.arg("-c").arg("filter.lfs.process=");
    command
}

fn git_text_with_index(root: &Path, args: &[&str], index: &Path) -> Result<String, CommitError> {
    let output = sanitized_command(root, index)
        .args(args)
        .output()
        .map_err(|error| CommitError::Git {
            command: args.join(" "),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(CommitError::Git {
            command: args.join(" "),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
