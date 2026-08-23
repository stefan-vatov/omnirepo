//! Isolated Git index preparation for the authorized operation delta.
//!
//! The operation stages its authorized changes into a private index file via
//! `GIT_INDEX_FILE` indirection: the frozen base-HEAD tree initializes the
//! index, and only the literal authorized paths enter — with exact blob
//! content hashed without filters, and hooks, attributes, fsmonitor, and
//! repository config unable to widen the staging.  The real index is never
//! touched, and a failure removes the isolated index and leaves no lock.

#![allow(dead_code)]

use super::state::{AuthorizedDelta, TargetChange};
use std::{error::Error, fmt, fs, path::Path, path::PathBuf, process::Command};

/// The prepared isolated index plus its owning temporary directory.
#[derive(Debug)]
pub struct IsolatedIndex {
    pub index_path: PathBuf,
    _temporary: OwnedTempDir,
}

/// Typed index-preparation failures.
#[derive(Debug)]
pub enum IndexError {
    Git { command: String, reason: String },
    UnsafePath { path: String },
    Io { path: PathBuf, reason: String },
}

impl fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git { command, reason } => {
                write!(
                    formatter,
                    "git index preparation failed ({command}): {reason}"
                )
            }
            Self::UnsafePath { path } => {
                write!(formatter, "index preparation rejected unsafe path {path:?}")
            }
            Self::Io { path, reason } => {
                write!(
                    formatter,
                    "index preparation io failure {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for IndexError {}

/// Prepare an isolated index that stages exactly the authorized delta.
pub fn prepare_index(root: &Path, delta: &AuthorizedDelta) -> Result<IsolatedIndex, IndexError> {
    let git_dir = root.join(git_text(root, &["rev-parse", "--git-dir"])?.trim());
    let temporary =
        OwnedTempDir::new(&git_dir, "omnirepo-index-").map_err(|error| IndexError::Io {
            path: git_dir.clone(),
            reason: error.to_string(),
        })?;
    let index_path = temporary.path().join("operation.index");

    // Build from the frozen committed baseline, never from the user's index.
    // An absent base-HEAD is the explicit unborn-repository case.
    if let Some(base_head) = delta.base_head() {
        run_git(root, &["read-tree", base_head.as_str()], Some(&index_path))?;
    } else {
        run_git(root, &["read-tree", "--empty"], Some(&index_path))?;
    }

    for change in delta.changes() {
        let path = String::from_utf8_lossy(change.target().path().as_bytes()).into_owned();
        validate_staging_path(&path)?;
        match change.change() {
            TargetChange::Deleted => {
                run_git(
                    root,
                    &["update-index", "--force-remove", "--", &path],
                    Some(&index_path),
                )?;
            }
            TargetChange::Added | TargetChange::Modified => {
                // Hash the exact working-tree bytes without filters and
                // register the literal authorized path in the isolated index.
                let worktree_file = root.join(&path);
                let blob = git_text_with_index(root, &["hash-object", "-w", "--", &path], None)?;
                run_git(
                    root,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{},{}", blob.trim(), path),
                    ],
                    Some(&index_path),
                )?;
                let _ = worktree_file;
            }
            TargetChange::Renamed => {
                let from = String::from_utf8_lossy(
                    change.rename_from().expect("rename source").as_bytes(),
                )
                .into_owned();
                validate_staging_path(&from)?;
                run_git(
                    root,
                    &["update-index", "--force-remove", "--", &from],
                    Some(&index_path),
                )?;
                let blob = git_text_with_index(root, &["hash-object", "-w", "--", &path], None)?;
                run_git(
                    root,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{},{}", blob.trim(), path),
                    ],
                    Some(&index_path),
                )?;
            }
            TargetChange::Untracked
            | TargetChange::TypeChanged
            | TargetChange::ModeChanged
            | TargetChange::LinkChanged => {
                // Type/mode/link changes re-register the path with the
                // working-tree blob; untracked is never authorized.
                if change.change() == TargetChange::Untracked {
                    return Err(IndexError::UnsafePath { path });
                }
                let blob = git_text_with_index(root, &["hash-object", "-w", "--", &path], None)?;
                run_git(
                    root,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{},{}", blob.trim(), path),
                    ],
                    Some(&index_path),
                )?;
            }
        }
    }
    Ok(IsolatedIndex {
        index_path,
        _temporary: temporary,
    })
}

/// An owned temporary directory created with a unique name below a parent;
/// removal happens on drop and never follows symlinks.
#[derive(Debug)]
struct OwnedTempDir {
    path: PathBuf,
}

impl OwnedTempDir {
    fn new(parent: &Path, prefix: &str) -> std::io::Result<Self> {
        let mut attempt = 0_u32;
        loop {
            let candidate = parent.join(format!("{prefix}{}-{}", std::process::id(), attempt));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self { path: candidate }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OwnedTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn validate_staging_path(path: &str) -> Result<(), IndexError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|component| component == "..")
    {
        return Err(IndexError::UnsafePath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn sanitized_command(root: &Path, index: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command.arg("--no-optional-locks");
    command.arg("-c").arg("core.hooksPath=/dev/null");
    command.arg("-c").arg("core.fsmonitor=false");
    command.arg("-c").arg("core.untrackedCache=false");
    command.arg("-c").arg("filter.lfs.smudge=");
    command.arg("-c").arg("filter.lfs.clean=");
    command.arg("-c").arg("filter.lfs.process=");
    command
}

fn run_git(root: &Path, args: &[&str], index: Option<&Path>) -> Result<(), IndexError> {
    let output = sanitized_command(root, index)
        .args(args)
        .output()
        .map_err(|error| IndexError::Io {
            path: root.to_path_buf(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(IndexError::Git {
            command: args.join(" "),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, IndexError> {
    let output = sanitized_command(root, None)
        .args(args)
        .output()
        .map_err(|error| IndexError::Io {
            path: root.to_path_buf(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(IndexError::Git {
            command: args.join(" "),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_text_with_index(
    root: &Path,
    args: &[&str],
    _index: Option<&Path>,
) -> Result<String, IndexError> {
    git_text(root, args)
}
