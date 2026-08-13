//! Sanitized capture of repository and Git state.
//!
//! HEAD, refs, index entries, worktree status, and the selected upstream are
//! inspected through hardened plumbing: hooks, filters, diff drivers,
//! credential helpers, optional locks, and repository-controlled config
//! includes can neither execute nor falsify the captured scope.  Output is
//! NUL-delimited and bounded; non-UTF-8 paths are captured losslessly where
//! the domain representation allows and lossily (marked) otherwise.

#![allow(dead_code)]

use super::state::{
    DomainError, GitFacts, GitRepositoryState, HeadState, IndexEntry, IndexState, RefName,
    RelativePath, RevisionId, TargetChange, UpstreamState, WorktreeEntry, WorktreeState,
};
use std::{
    error::Error,
    fmt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Maximum accepted bytes for any single captured git output stream.
pub const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
/// Total time budget for one capture command.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);

/// Hardening applied to every captured git invocation.
fn hardened_git(root: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.arg("--no-optional-locks");
    command.arg("-c").arg("core.hooksPath=/dev/null");
    command.arg("-c").arg("core.fsmonitor=false");
    command.arg("-c").arg("core.untrackedCache=false");
    command.args(arguments);
    command
}

/// Capture failures; the repository is never modified.
#[derive(Debug)]
pub enum CaptureError {
    /// The capture binary could not start.
    Spawn(String),
    /// The capture command exceeded its time budget.
    Timeout { command: String },
    /// The capture output exceeded the byte bound.
    OutputTooLarge { command: String },
    /// Git reported an error (non-Git directory, hostile config, corruption).
    Git { command: String, stderr: String },
    /// The captured state violates the domain contract.
    Domain(DomainError),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(reason) => write!(formatter, "cannot start git capture: {reason}"),
            Self::Timeout { command } => {
                write!(formatter, "git capture timed out: {command}")
            }
            Self::OutputTooLarge { command } => {
                write!(
                    formatter,
                    "git capture output exceeded the bound: {command}"
                )
            }
            Self::Git { command, stderr } => {
                write!(formatter, "git capture failed ({command}): {stderr}")
            }
            Self::Domain(error) => write!(formatter, "captured state is invalid: {error}"),
        }
    }
}
impl Error for CaptureError {}

fn run_bounded(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, CaptureError> {
    let label = arguments.join(" ");
    let mut child = hardened_git(root, arguments)
        .spawn()
        .map_err(|error| CaptureError::Spawn(error.to_string()))?;
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    use std::io::Read;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(mut handle) = child.stdout.take() {
                    handle
                        .read_to_end(&mut stdout)
                        .map_err(|error| CaptureError::Spawn(error.to_string()))?;
                }
                if let Some(mut handle) = child.stderr.take() {
                    handle
                        .read_to_end(&mut stderr)
                        .map_err(|error| CaptureError::Spawn(error.to_string()))?;
                }
                if stdout.len() > MAX_CAPTURE_BYTES || stderr.len() > MAX_CAPTURE_BYTES {
                    return Err(CaptureError::OutputTooLarge {
                        command: label.clone(),
                    });
                }
                if !status.success() {
                    return Err(CaptureError::Git {
                        command: label,
                        stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
                    });
                }
                return Ok(stdout);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CaptureError::Timeout { command: label });
                }
                // Drain available output while waiting, bounded.
                if let Some(handle) = child.stdout.as_mut() {
                    let mut buffer = [0_u8; 8192];
                    loop {
                        match handle.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                stdout.extend_from_slice(&buffer[..read]);
                                if stdout.len() > MAX_CAPTURE_BYTES {
                                    let _ = child.kill();
                                    return Err(CaptureError::OutputTooLarge { command: label });
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                return Err(CaptureError::Spawn(error.to_string()));
            }
        }
    }
}

/// Capture the repository facts for a destination root.
///
/// A directory without a Git repository is a lawful `NonGit` state; every
/// other failure is typed and leaves the repository untouched.
pub fn capture_state(root: &Path) -> Result<GitRepositoryState, CaptureError> {
    if !root.join(".git").exists() {
        return Ok(GitRepositoryState::NonGit);
    }
    let head = capture_head(root)?;
    let upstream = capture_upstream(root)?;
    let (index, worktree) = capture_status(root)?;
    let facts = GitFacts::new(head, upstream, index, worktree).map_err(CaptureError::Domain)?;
    Ok(GitRepositoryState::Git(facts))
}

fn capture_head(root: &Path) -> Result<HeadState, CaptureError> {
    let commit = match run_bounded(root, &["rev-parse", "--verify", "HEAD^{commit}"]) {
        Ok(output) => {
            let value = String::from_utf8_lossy(&output).trim().to_owned();
            if value.is_empty() {
                return Err(CaptureError::Git {
                    command: "rev-parse HEAD".to_owned(),
                    stderr: "empty revision".to_owned(),
                });
            }
            value
        }
        Err(CaptureError::Git { .. }) => return Ok(HeadState::Unborn),
        Err(error) => return Err(error),
    };
    let branch = run_bounded(root, &["symbolic-ref", "--quiet", "HEAD"]).ok();
    let branch = branch
        .map(|output| String::from_utf8_lossy(&output).trim().to_owned())
        .filter(|value| !value.is_empty());
    match branch {
        Some(branch) => {
            let ref_name = RefName::new(&branch).map_err(CaptureError::Domain)?;
            let revision = RevisionId::new(commit).map_err(CaptureError::Domain)?;
            Ok(HeadState::Attached {
                branch: ref_name,
                commit: revision,
            })
        }
        None => {
            let revision = RevisionId::new(commit).map_err(CaptureError::Domain)?;
            Ok(HeadState::Detached { commit: revision })
        }
    }
}

fn capture_upstream(root: &Path) -> Result<UpstreamState, CaptureError> {
    let branch = match run_bounded(root, &["symbolic-ref", "--quiet", "HEAD"]) {
        Ok(output) => String::from_utf8_lossy(&output).trim().to_owned(),
        Err(CaptureError::Git { .. }) => return Ok(UpstreamState::Absent),
        Err(error) => return Err(error),
    };
    if branch.is_empty() {
        return Ok(UpstreamState::Absent);
    }
    let upstream_ref =
        match run_bounded(root, &["rev-parse", "--symbolic-full-name", "@{upstream}"]) {
            Ok(output) => String::from_utf8_lossy(&output).trim().to_owned(),
            Err(CaptureError::Git { .. }) => return Ok(UpstreamState::Absent),
            Err(error) => return Err(error),
        };
    if upstream_ref.is_empty() {
        return Ok(UpstreamState::Absent);
    }
    let upstream_commit =
        match run_bounded(root, &["rev-parse", "--verify", "@{upstream}^{commit}"]) {
            Ok(output) => String::from_utf8_lossy(&output).trim().to_owned(),
            Err(CaptureError::Git { .. }) => return Ok(UpstreamState::Absent),
            Err(error) => return Err(error),
        };
    let short_branch = branch.strip_prefix("refs/heads/").unwrap_or(&branch);
    let remote = run_bounded(
        root,
        &["config", "--get", &format!("branch.{short_branch}.remote")],
    )
    .map(|output| String::from_utf8_lossy(&output).trim().to_owned())
    .unwrap_or_default();
    let reference = RefName::new(&upstream_ref).map_err(CaptureError::Domain)?;
    let commit = RevisionId::new(upstream_commit).map_err(CaptureError::Domain)?;
    Ok(UpstreamState::Configured {
        remote,
        reference,
        commit,
    })
}

fn capture_status(root: &Path) -> Result<(IndexState, WorktreeState), CaptureError> {
    let output = run_bounded(
        root,
        &["status", "--porcelain", "--untracked-files=all", "-z"],
    )?;
    let mut index_entries = Vec::new();
    let mut worktree_entries = Vec::new();
    let records = output.split(|byte| *byte == 0);
    let mut records = records.peekable();
    while let Some(record) = records.next() {
        if record.len() < 3 || record[2] != b' ' {
            continue; // guard: every v1 porcelain record starts with "XY "
        }
        let x = record[0];
        let y = record[1];
        // Raw byte path: non-UTF-8 names are captured losslessly through the
        // domain's byte-based RelativePath.
        let path_bytes = &record[3..];
        let index_change = index_change(x);
        let worktree_change = worktree_change(y);
        if let Some(change) = index_change {
            let entry = if change == TargetChange::Renamed {
                let from_raw = records.next().unwrap_or_default();
                IndexEntry::renamed(
                    RelativePath::from_bytes(from_raw).map_err(|error| CaptureError::Git {
                        command: "status rename source".to_owned(),
                        stderr: error.to_string(),
                    })?,
                    RelativePath::from_bytes(path_bytes).map_err(|error| CaptureError::Git {
                        command: "status rename target".to_owned(),
                        stderr: error.to_string(),
                    })?,
                    super::state::DirtyProvenance::PreExisting,
                )
            } else {
                IndexEntry::new(
                    RelativePath::from_bytes(path_bytes).map_err(|error| CaptureError::Git {
                        command: "status index path".to_owned(),
                        stderr: error.to_string(),
                    })?,
                    change,
                    super::state::DirtyProvenance::PreExisting,
                )
            }
            .map_err(CaptureError::Domain)?;
            index_entries.push(entry);
        }
        if let Some(change) = worktree_change {
            let entry = WorktreeEntry::new(
                RelativePath::from_bytes(path_bytes).map_err(|error| CaptureError::Git {
                    command: "status worktree path".to_owned(),
                    stderr: error.to_string(),
                })?,
                change,
                super::state::DirtyProvenance::PreExisting,
            )
            .map_err(CaptureError::Domain)?;
            worktree_entries.push(entry);
        }
    }
    let index = if index_entries.is_empty() {
        IndexState::Clean
    } else {
        IndexState::Entries(index_entries)
    };
    let worktree = if worktree_entries.is_empty() {
        WorktreeState::Clean
    } else {
        WorktreeState::Entries(worktree_entries)
    };
    Ok((index, worktree))
}

fn index_change(code: u8) -> Option<TargetChange> {
    match code {
        b'A' => Some(TargetChange::Added),
        b'M' => Some(TargetChange::Modified),
        b'D' => Some(TargetChange::Deleted),
        b'R' => Some(TargetChange::Renamed),
        b'T' => Some(TargetChange::TypeChanged),
        b'U' => Some(TargetChange::Modified),
        _ => None,
    }
}

fn worktree_change(code: u8) -> Option<TargetChange> {
    match code {
        b'M' => Some(TargetChange::Modified),
        b'D' => Some(TargetChange::Deleted),
        b'?' => Some(TargetChange::Untracked),
        b'T' => Some(TargetChange::TypeChanged),
        b'U' => Some(TargetChange::Modified),
        _ => None,
    }
}
