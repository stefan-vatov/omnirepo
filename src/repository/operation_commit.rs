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
    // The sanitized environment drops every config file, so the invoking
    // user's identity and signing policy are resolved explicitly through
    // the normal config chain (local, then global/XDG) and passed to the
    // commit.  Missing identity is a repository failure, never a bypass.
    let identity = resolve_user_identity(root)?;
    let mut arguments = vec!["commit-tree".to_owned()];
    // commit-tree ignores the commit.gpgsign config, so the user's signing
    // policy is applied explicitly with -S (and the configured key).
    if identity.gpgsign {
        if let Some(key) = &identity.signing_key {
            arguments.push(format!("-S{key}"));
        } else {
            arguments.push("-S".to_owned());
        }
    }
    arguments.push(tree.clone());
    if let Some(parent) = parent {
        arguments.push("-p".to_owned());
        arguments.push(parent.to_owned());
    }
    arguments.push("-m".to_owned());
    arguments.push(message.to_owned());
    let mut command = sanitized_command(root, &index.index_path);
    command
        .env("GIT_AUTHOR_NAME", &identity.name)
        .env("GIT_AUTHOR_EMAIL", &identity.email)
        .env("GIT_COMMITTER_NAME", &identity.name)
        .env("GIT_COMMITTER_EMAIL", &identity.email)
        .args(&arguments);
    let output = command.output().map_err(|error| CommitError::Git {
        command: "commit-tree".to_owned(),
        reason: error.to_string(),
    })?;
    if !output.status.success() {
        return Err(CommitError::Git {
            command: arguments.join(" "),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
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

/// The invoking user's resolved Git identity and signing policy.
struct UserIdentity {
    name: String,
    email: String,
    signing_key: Option<String>,
    gpgsign: bool,
}

/// Resolve a config value through the normal chain (local, then global and
/// XDG) from the destination root.  The read never runs hooks or filters.
fn user_config(root: &Path, key: &str) -> Result<Option<String>, CommitError> {
    let output = Command::new("git")
        .args(["config", key])
        .current_dir(root)
        .output()
        .map_err(|error| CommitError::Git {
            command: format!("git config {key}"),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(if value.is_empty() { None } else { Some(value) })
}

/// Resolve a config value in one explicit scope.  Used for the signing
/// policy so it always comes from the same source as the identity that
/// actually won the chain.
fn scoped_config(root: &Path, scope: &str, key: &str) -> Result<Option<String>, CommitError> {
    let output = Command::new("git")
        .args(["config", scope, key])
        .current_dir(root)
        .output()
        .map_err(|error| CommitError::Git {
            command: format!("git config {scope} {key}"),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(if value.is_empty() { None } else { Some(value) })
}

/// Resolve the identity through the normal chain and report whether the
/// winning source was the repository-local config.
fn identity_with_origin(root: &Path, key: &str) -> Result<(Option<String>, bool), CommitError> {
    let output = Command::new("git")
        .args(["config", "--show-origin", key])
        .current_dir(root)
        .output()
        .map_err(|error| CommitError::Git {
            command: format!("git config --show-origin {key}"),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok((None, false));
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok((None, false));
    }
    let (origin, value) = match trimmed.split_once('\t') {
        Some((origin, value)) => (origin, value.to_owned()),
        None => ("", trimmed.to_owned()),
    };
    let local = origin.contains(".git/config") || origin.contains("\\.git\\config");
    Ok((Some(value), local))
}

fn resolve_user_identity(root: &Path) -> Result<UserIdentity, CommitError> {
    let (name, local_identity) = identity_with_origin(root, "user.name")?;
    let name = name.ok_or_else(|| CommitError::Git {
        command: "git config user.name".to_owned(),
        reason: "author identity unknown: the invoking user has no configured Git identity"
            .to_owned(),
    })?;
    let (email, _) = identity_with_origin(root, "user.email")?;
    let email = email.ok_or_else(|| CommitError::Git {
        command: "git config user.email".to_owned(),
        reason: "author email unknown: the invoking user has no configured Git email".to_owned(),
    })?;
    // The signing policy comes from the same source that supplied the
    // identity: a repository-declared identity (fixture or repo-local)
    // carries its own signing scope; otherwise the invoking user's global
    // and XDG policy applies.
    let (signing_key, gpgsign) = if local_identity {
        (
            scoped_config(root, "--local", "user.signingkey")?,
            scoped_config(root, "--local", "commit.gpgsign")?
                .map(|value| value == "true")
                .unwrap_or(false),
        )
    } else {
        (
            scoped_config(root, "--global", "user.signingkey")?,
            scoped_config(root, "--global", "commit.gpgsign")?
                .map(|value| value == "true")
                .unwrap_or(false),
        )
    };
    Ok(UserIdentity {
        name,
        email,
        signing_key,
        gpgsign,
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
