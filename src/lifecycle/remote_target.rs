//! Freeze and validate the exact remote publication target.
//!
//! One canonical remote/ref/OID tuple is recorded before any remote
//! contact: the upstream resolves to exactly one remote name, one
//! `refs/heads/*` reference, and one locally existing OID.  Detached,
//! unborn, and no-upstream heads fail typed; ahead/behind/diverged counts
//! are computed exactly; an unsanitizable transport (non-https/ssh scheme
//! or embedded credentials) fails before contact.

#![allow(dead_code)]

use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{GitRepositoryState, HeadState, RefName, RevisionId, UpstreamState};

#[cfg(test)]
mod remote_target_tests;

use std::{error::Error, fmt, path::Path, process::Command};

/// The frozen canonical publication target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenRemoteTarget {
    pub remote: String,
    pub reference: RefName,
    pub oid: RevisionId,
}

/// Exact publication posture relative to the frozen remote target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicationPosture {
    /// Local and remote are at the same OID; nothing to publish.
    InSync { target: FrozenRemoteTarget },
    /// Local is strictly ahead; a push publishes exactly the frozen target.
    Ahead {
        target: FrozenRemoteTarget,
        ahead: u64,
    },
    /// Local is behind the remote; publication must not overwrite remote
    /// history.
    Behind {
        target: FrozenRemoteTarget,
        behind: u64,
    },
    /// Both sides advanced; publication is ambiguous.
    Diverged {
        target: FrozenRemoteTarget,
        ahead: u64,
        behind: u64,
    },
}

/// Freeze failures; nothing is contacted on failure.
#[derive(Debug)]
pub enum RemoteTargetError {
    Detached { reason: String },
    NoUpstream { reason: String },
    Unborn { reason: String },
    TransportUnsanitized { reason: String },
    Git { command: String, reason: String },
    Authority { reason: String },
}

impl fmt::Display for RemoteTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Detached { reason } => {
                write!(formatter, "publication target is detached: {reason}")
            }
            Self::NoUpstream { reason } => {
                write!(formatter, "publication target has no upstream: {reason}")
            }
            Self::Unborn { reason } => {
                write!(formatter, "publication target is unborn: {reason}")
            }
            Self::TransportUnsanitized { reason } => write!(
                formatter,
                "publication transport cannot be sanitized before contact: {reason}"
            ),
            Self::Git { command, reason } => {
                write!(
                    formatter,
                    "publication target freeze failed ({command}): {reason}"
                )
            }
            Self::Authority { reason } => {
                write!(formatter, "publication target authority failed: {reason}")
            }
        }
    }
}
impl Error for RemoteTargetError {}

/// Freeze the exact remote publication target under the typed Git working
/// directory root (no-follow authority), validating the tuple and the
/// transport without contacting the remote.
pub fn freeze_remote_target(
    root: &AuthorityRoot<GitWorkingDirectoryRoot, ReadOnly>,
) -> Result<(FrozenRemoteTarget, PublicationPosture), RemoteTargetError> {
    let working = root.display_path().as_path();
    let captured =
        crate::repository::capture_state(working).map_err(|error| RemoteTargetError::Git {
            command: "capture-state".to_owned(),
            reason: error.to_string(),
        })?;
    let GitRepositoryState::Git(facts) = &captured else {
        return Err(RemoteTargetError::NoUpstream {
            reason: "the destination is not a Git repository".to_owned(),
        });
    };
    match facts.head() {
        HeadState::Unborn => {
            return Err(RemoteTargetError::Unborn {
                reason: "the current branch has no commit".to_owned(),
            });
        }
        HeadState::Detached { .. } => {
            return Err(RemoteTargetError::Detached {
                reason: "publication requires an attached branch".to_owned(),
            });
        }
        HeadState::Attached { .. } => {}
    }
    let upstream = match facts.upstream() {
        UpstreamState::Configured {
            remote,
            reference,
            commit,
        } => (remote.clone(), reference.clone(), commit.clone()),
        UpstreamState::Absent => {
            return Err(RemoteTargetError::NoUpstream {
                reason: "no upstream is configured for the current branch".to_owned(),
            });
        }
    };
    let (remote, reference, oid) = upstream;
    let reference_name = reference.as_str();
    if !reference_name.starts_with("refs/remotes/") {
        return Err(RemoteTargetError::NoUpstream {
            reason: format!("upstream reference {reference_name:?} is not a remote-tracking ref"),
        });
    }
    // The canonical remote-side reference is the merge ref of the local
    // branch (refs/heads/*), never the tracking ref.
    let local_branch =
        git_text(working, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(|error| {
            RemoteTargetError::Git {
                command: "rev-parse --abbrev-ref HEAD".to_owned(),
                reason: error,
            }
        })?;
    let local_branch = local_branch.trim().to_owned();
    let merge = git_text(
        working,
        &["config", "--get", &format!("branch.{local_branch}.merge")],
    )
    .map_err(|error| RemoteTargetError::NoUpstream {
        reason: format!("no merge ref for branch {local_branch:?}: {error}"),
    })?;
    let merge = merge.trim().to_owned();
    if !merge.starts_with("refs/heads/") {
        return Err(RemoteTargetError::NoUpstream {
            reason: format!("merge ref {merge:?} is not a branch ref"),
        });
    }
    let reference = RefName::new(&merge).map_err(|error| RemoteTargetError::NoUpstream {
        reason: format!("invalid merge ref {merge:?}: {error}"),
    })?;
    // The frozen OID must exist locally (reconciliation against the object
    // database), else the tuple cannot be recorded.
    let exists = git_text(working, &["cat-file", "-e", oid.as_str()]).is_ok();
    if !exists {
        return Err(RemoteTargetError::NoUpstream {
            reason: format!("upstream OID {} does not exist locally", oid.as_str()),
        });
    }
    let target = FrozenRemoteTarget {
        remote: remote.clone(),
        reference: reference.clone(),
        oid: oid.clone(),
    };
    // Transport sanitization: only https/ssh, and no embedded credentials.
    let remote_url = git_text(working, &["remote", "get-url", &remote]).map_err(|error| {
        RemoteTargetError::Git {
            command: format!("remote get-url {remote}"),
            reason: error,
        }
    })?;
    sanitize_transport(&remote_url, &remote)?;
    // Exact ahead/behind counts.
    let ahead_text =
        git_text(working, &["rev-list", "--count", "@{u}..HEAD"]).map_err(|error| {
            RemoteTargetError::Git {
                command: "rev-list --count @{u}..HEAD".to_owned(),
                reason: error,
            }
        })?;
    let behind_text =
        git_text(working, &["rev-list", "--count", "HEAD..@{u}"]).map_err(|error| {
            RemoteTargetError::Git {
                command: "rev-list --count HEAD..@{u}".to_owned(),
                reason: error,
            }
        })?;
    let ahead: u64 = ahead_text
        .trim()
        .parse::<u64>()
        .map_err(|error| RemoteTargetError::Git {
            command: "parse ahead count".to_owned(),
            reason: error.to_string(),
        })?;
    let behind: u64 =
        behind_text
            .trim()
            .parse::<u64>()
            .map_err(|error| RemoteTargetError::Git {
                command: "parse behind count".to_owned(),
                reason: error.to_string(),
            })?;
    let posture = match (ahead, behind) {
        (0, 0) => PublicationPosture::InSync {
            target: target.clone(),
        },
        (ahead, 0) => PublicationPosture::Ahead {
            target: target.clone(),
            ahead,
        },
        (0, behind) => PublicationPosture::Behind {
            target: target.clone(),
            behind,
        },
        (ahead, behind) => PublicationPosture::Diverged {
            target: target.clone(),
            ahead,
            behind,
        },
    };
    Ok((target, posture))
}

/// Reject any transport that cannot be sanitized before contact: only
/// https and ssh schemes, and no embedded credentials.
fn sanitize_transport(url: &str, remote: &str) -> Result<(), RemoteTargetError> {
    let scheme = url.split("://").next().unwrap_or("");
    match scheme {
        "https" | "ssh" => {}
        "http" => {
            return Err(RemoteTargetError::TransportUnsanitized {
                reason: format!("remote {remote} uses plaintext http"),
            });
        }
        _ => {
            return Err(RemoteTargetError::TransportUnsanitized {
                reason: format!("remote {remote} scheme {scheme:?} is not https or ssh"),
            });
        }
    }
    let authority_part = url
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    // A bare user (git@host) is the normal ssh form; credentials embed a
    // password or token, which the userinfo must never carry.
    if let Some((userinfo, _host)) = authority_part.rsplit_once('@') {
        if userinfo.contains(':') {
            return Err(RemoteTargetError::TransportUnsanitized {
                reason: format!("remote {remote} embeds credentials in its URL"),
            });
        }
    }
    Ok(())
}

/// The hardened git text command (bounded, inert hooks/config, no
/// optional locks), identical to the repository-domain pattern.
pub(crate) fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = sanitized_command(root)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn sanitized_command(root: &Path) -> Command {
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
        .arg("core.untrackedCache=false")
        .arg("-c")
        .arg("filter.lfs.smudge=")
        .arg("-c")
        .arg("filter.lfs.clean=")
        .arg("-c")
        .arg("filter.lfs.process=");
    command
}
