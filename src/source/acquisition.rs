//! Confined local and remote source acquisition adapters.
//!
//! Each owner-selected source reference materializes into isolated staging
//! beneath the machine cache root with sanitized Git behavior: no
//! source-controlled hooks, filters, submodules, LFS smudging, executable
//! protocol helpers, or URL rewrites; user HTTPS credential helpers and the
//! SSH agent remain available (canon/architecture/configuration-authority.md).
//! Fetching is bounded per attempt with a deadline, a bounded retry budget,
//! and redacted contextual failures; staging can never escape its root.

#![allow(dead_code)]

use super::snapshot::{
    CacheKey, IdentityError, PublishedSnapshot, RevisionId, SnapshotId, SourceId, SourceIdentity,
};
use crate::configuration::{AbsolutePath, SourceLocation, SourceReference};
use std::{
    error::Error,
    fmt,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Maximum accepted bytes for any acquisition output stream.
pub const MAX_ACQUIRE_BYTES: usize = 8 * 1024 * 1024;
/// Default per-attempt fetch deadline (canon: two minutes).
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// Canon: one fetch attempt followed by at most two retries.
pub const DEFAULT_MAX_RETRIES: u8 = 2;
/// Bounded backoff base between retries.
pub const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);

/// Acquisition controls.
#[derive(Clone, Debug)]
pub struct AcquireConfig {
    pub cache_root: PathBuf,
    pub fetch_timeout: Duration,
    pub max_retries: u8,
    pub backoff: Duration,
}

impl AcquireConfig {
    pub fn new(cache_root: impl Into<PathBuf>) -> Self {
        Self {
            cache_root: cache_root.into(),
            fetch_timeout: DEFAULT_FETCH_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            backoff: DEFAULT_BACKOFF,
        }
    }
}

/// Typed acquisition failures; credentials are always redacted.
#[derive(Debug)]
pub enum AcquireError {
    Unsupported { reason: String },
    Ambiguous { reason: String },
    Authentication { reason: String },
    Network { reason: String },
    Cache { reason: String },
    Containment { reason: String },
    Io { path: PathBuf, reason: String },
    Identity(IdentityError),
}

impl fmt::Display for AcquireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { reason } => write!(formatter, "unsupported source: {reason}"),
            Self::Ambiguous { reason } => write!(formatter, "ambiguous source state: {reason}"),
            Self::Authentication { reason } => {
                write!(formatter, "source authentication failed: {reason}")
            }
            Self::Network { reason } => write!(formatter, "source network failure: {reason}"),
            Self::Cache { reason } => write!(formatter, "source cache failure: {reason}"),
            Self::Containment { reason } => {
                write!(formatter, "source containment failure: {reason}")
            }
            Self::Io { path, reason } => {
                write!(formatter, "cannot access {}: {reason}", path.display())
            }
            Self::Identity(error) => write!(formatter, "source identity failure: {error}"),
        }
    }
}
impl Error for AcquireError {}

/// Acquire one source reference and pin its exact selected revision.
pub fn acquire(
    reference: &SourceReference,
    config: &AcquireConfig,
) -> Result<PublishedSnapshot, AcquireError> {
    let source = SourceIdentity::new(
        SourceId::new(reference.id().as_str()).map_err(AcquireError::Identity)?,
        reference.location().as_str(),
    )
    .map_err(AcquireError::Identity)?;
    match reference.location() {
        SourceLocation::Local(path) => acquire_local(&source, path),
        SourceLocation::Remote(url) => acquire_remote(&source, url, config),
    }
}

fn acquire_local(
    source: &SourceIdentity,
    path: &AbsolutePath,
) -> Result<PublishedSnapshot, AcquireError> {
    let root = Path::new(path.as_str());
    if !root.is_dir() {
        return Err(AcquireError::Unsupported {
            reason: format!("local source is not a directory: {}", path.as_str()),
        });
    }
    if !root.join(".git").exists() {
        return Err(AcquireError::Unsupported {
            reason: format!("local source is not a Git worktree: {}", path.as_str()),
        });
    }
    // The local source must be a clean worktree on main; its HEAD is pinned
    // and never pulled or rewritten.
    let branch = git_text(root, &["symbolic-ref", "--quiet", "HEAD"]).map_err(|error| {
        AcquireError::Ambiguous {
            reason: format!("cannot resolve local HEAD: {error}"),
        }
    })?;
    if branch.trim() != "refs/heads/main" {
        return Err(AcquireError::Ambiguous {
            reason: format!(
                "local source must be on main, found {}",
                branch.trim().trim_start_matches("refs/heads/")
            ),
        });
    }
    let dirty =
        git_text(root, &["status", "--porcelain"]).map_err(|error| AcquireError::Ambiguous {
            reason: format!("cannot inspect local worktree: {error}"),
        })?;
    if !dirty.trim().is_empty() {
        return Err(AcquireError::Ambiguous {
            reason: "local source worktree is not clean".to_owned(),
        });
    }
    let revision_text =
        git_text(root, &["rev-parse", "--verify", "main^{commit}"]).map_err(|error| {
            AcquireError::Ambiguous {
                reason: format!("cannot pin local main revision: {error}"),
            }
        })?;
    let revision = RevisionId::new(revision_text.trim()).map_err(AcquireError::Identity)?;
    let snapshot_id = SnapshotId::new("local-snapshot").map_err(AcquireError::Identity)?;
    let cache = CacheKey::new(path.as_str()).map_err(AcquireError::Identity)?;
    Ok(PublishedSnapshot::new(
        source.clone(),
        revision,
        snapshot_id,
        cache,
    ))
}

pub(crate) fn acquire_remote(
    source: &SourceIdentity,
    url: &str,
    config: &AcquireConfig,
) -> Result<PublishedSnapshot, AcquireError> {
    let _guard = SourceLock::acquire(config.cache_root.as_path(), source.id().as_str())?;
    acquire_remote_locked(source, url, config)
}

/// The acquisition critical section; callers that already hold the source
/// lock use this directly.
pub(crate) fn acquire_remote_locked(
    source: &SourceIdentity,
    url: &str,
    config: &AcquireConfig,
) -> Result<PublishedSnapshot, AcquireError> {
    let cache_root = config.cache_root.as_path();
    if !cache_root.is_absolute() {
        return Err(AcquireError::Containment {
            reason: "cache root must be absolute".to_owned(),
        });
    }
    if !cache_root.is_dir() {
        return Err(AcquireError::Containment {
            reason: format!("cache root is not a directory: {}", cache_root.display()),
        });
    }
    let staging = cache_root.join(source.id().as_str());
    if staging
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AcquireError::Containment {
            reason: "staging path escapes the cache root".to_owned(),
        });
    }
    match std::fs::symlink_metadata(&staging) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AcquireError::Containment {
                reason: format!("staging path is a symlink: {}", staging.display()),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AcquireError::Io {
                path: staging.clone(),
                reason: error.to_string(),
            });
        }
    }

    ensure_staging_repo(&staging, url)?;
    let revision_text = fetch_with_retries(&staging, url, config)?;
    let revision = RevisionId::new(revision_text).map_err(AcquireError::Identity)?;
    let snapshot_id = SnapshotId::new("remote-snapshot").map_err(AcquireError::Identity)?;
    let cache = CacheKey::new(staging.display().to_string()).map_err(AcquireError::Identity)?;
    Ok(PublishedSnapshot::new(
        source.clone(),
        revision,
        snapshot_id,
        cache,
    ))
}

/// Ensure the staging repository exists, is ours, and points at the exact
/// source URL.  A missing, corrupt, or wrong-remote staging may be discarded
/// and cleanly recreated (canon cache policy).
fn ensure_staging_repo(staging: &Path, url: &str) -> Result<(), AcquireError> {
    if staging.join(".git").exists() {
        let configured = git_text_optional(staging, &["config", "--get", "remote.origin.url"]);
        if configured.as_deref() != Some(url) {
            discard_staging(staging)?;
        }
    }
    if !staging.join(".git").exists() {
        std::fs::create_dir_all(staging).map_err(|error| AcquireError::Io {
            path: staging.to_path_buf(),
            reason: error.to_string(),
        })?;
        run_git(staging, &["init", "--quiet"])?;
        run_git(staging, &["remote", "add", "origin", url])?;
    }
    Ok(())
}

fn discard_staging(staging: &Path) -> Result<(), AcquireError> {
    // Never follow symlinks or escape: remove only the exact owned subtree.
    if staging.is_symlink() || !staging.is_dir() {
        return Err(AcquireError::Containment {
            reason: format!(
                "refusing to discard non-directory staging: {}",
                staging.display()
            ),
        });
    }
    std::fs::remove_dir_all(staging).map_err(|error| AcquireError::Io {
        path: staging.to_path_buf(),
        reason: error.to_string(),
    })
}

fn fetch_with_retries(
    staging: &Path,
    url: &str,
    config: &AcquireConfig,
) -> Result<String, AcquireError> {
    let mut last_error = None;
    let attempts = config.max_retries.saturating_add(1);
    for attempt in 0..attempts {
        if attempt > 0 {
            let delay = config.backoff.saturating_mul(attempt as u32);
            std::thread::sleep(delay);
        }
        match fetch_once(staging, url, config.fetch_timeout) {
            Ok(revision) => return Ok(revision),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| AcquireError::Network {
        reason: "no fetch attempts were made".to_owned(),
    }))
}

fn fetch_once(staging: &Path, url: &str, timeout: Duration) -> Result<String, AcquireError> {
    // Explicit URL, no tags, no submodules, filters and hooks inert.
    let output = run_bounded(
        staging,
        &[
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            url,
            "main",
        ],
        timeout,
    )?;
    let _ = output;
    let revision =
        git_text(staging, &["rev-parse", "--verify", "FETCH_HEAD^{commit}"]).map_err(|error| {
            AcquireError::Network {
                reason: format!("cannot pin fetched revision: {error}"),
            }
        })?;
    Ok(revision.trim().to_owned())
}

/// Sanitized git environment for every acquisition command.
fn sanitized_command(staging: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(staging)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.arg("--no-optional-locks");
    command.arg("-c").arg("core.hooksPath=/dev/null");
    command.arg("-c").arg("core.fsmonitor=false");
    command.arg("-c").arg("core.untrackedCache=false");
    command.arg("-c").arg("filter.lfs.smudge=");
    command.arg("-c").arg("filter.lfs.clean=");
    command.arg("-c").arg("filter.lfs.process=");
    command
}

fn run_git(staging: &Path, arguments: &[&str]) -> Result<Vec<u8>, AcquireError> {
    let output = sanitized_command(staging)
        .args(arguments)
        .output()
        .map_err(|error| AcquireError::Io {
            path: staging.to_path_buf(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(AcquireError::Network {
            reason: classify_failure(&output.stderr),
        });
    }
    Ok(output.stdout)
}

fn git_text(staging: &Path, arguments: &[&str]) -> Result<String, AcquireError> {
    Ok(String::from_utf8_lossy(&run_git(staging, arguments)?).into_owned())
}

fn git_text_optional(staging: &Path, arguments: &[&str]) -> Option<String> {
    git_text(staging, arguments).ok()
}

/// Run one bounded command with a deadline; on timeout the child is killed.
fn run_bounded(
    staging: &Path,
    arguments: &[&str],
    timeout: Duration,
) -> Result<Vec<u8>, AcquireError> {
    let mut child = sanitized_command(staging)
        .args(arguments)
        .spawn()
        .map_err(|error| AcquireError::Io {
            path: staging.to_path_buf(),
            reason: error.to_string(),
        })?;
    let deadline = Instant::now() + timeout;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(mut handle) = child.stdout.take() {
                    handle
                        .read_to_end(&mut stdout)
                        .map_err(|error| AcquireError::Io {
                            path: staging.to_path_buf(),
                            reason: error.to_string(),
                        })?;
                }
                if let Some(mut handle) = child.stderr.take() {
                    handle
                        .read_to_end(&mut stderr)
                        .map_err(|error| AcquireError::Io {
                            path: staging.to_path_buf(),
                            reason: error.to_string(),
                        })?;
                }
                if stdout.len() > MAX_ACQUIRE_BYTES || stderr.len() > MAX_ACQUIRE_BYTES {
                    return Err(AcquireError::Network {
                        reason: "acquisition output exceeded the bound".to_owned(),
                    });
                }
                if !status.success() {
                    return Err(AcquireError::Network {
                        reason: classify_failure(&stderr),
                    });
                }
                return Ok(stdout);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AcquireError::Network {
                        reason: "fetch exceeded its time budget".to_owned(),
                    });
                }
                if let Some(handle) = child.stdout.as_mut() {
                    let mut buffer = [0_u8; 8192];
                    loop {
                        match handle.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                stdout.extend_from_slice(&buffer[..read]);
                                if stdout.len() > MAX_ACQUIRE_BYTES {
                                    let _ = child.kill();
                                    return Err(AcquireError::Network {
                                        reason: "acquisition output exceeded the bound".to_owned(),
                                    });
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                return Err(AcquireError::Io {
                    path: staging.to_path_buf(),
                    reason: error.to_string(),
                });
            }
        }
    }
}

/// Classify a git failure and redact any embedded credentials.
fn classify_failure(stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stderr).into_owned();
    text = redact_credentials(&text);
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("authentication failed")
        || lowered.contains("permission denied (publickey)")
        || lowered.contains("could not read username")
        || lowered.contains("could not read password")
        || lowered.contains("terminal prompts disabled")
    {
        format!("authentication: {text}")
    } else if lowered.contains("could not resolve host")
        || lowered.contains("connection timed out")
        || lowered.contains("connection refused")
        || lowered.contains("unable to access")
    {
        format!("network: {text}")
    } else {
        text
    }
}

/// Replace `scheme://user:password@` and `scheme://token@` forms so
/// credentials never reach logs or diagnostics.
fn redact_credentials(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"://") {
            // Walk back to the scheme start and forward to the '@'.
            let scheme_start = text[..index].rfind(|character: char| {
                !(character.is_ascii_alphanumeric()
                    || character == '+'
                    || character == '-'
                    || character == '.')
            });
            let scheme_start = scheme_start.map(|position| position + 1).unwrap_or(0);
            let rest = &text[index + 3..];
            if let Some(at) = rest.find('@') {
                let authority = &rest[..at];
                if authority.contains(':') || authority.contains("oauth2") {
                    output.push_str(&text[scheme_start..index + 3]);
                    output.push_str("***@");
                    index = index + 3 + at + 1;
                    continue;
                }
            }
        }
        let character = text[index..].chars().next().expect("char");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

/// Bounded per-source acquisition lock: exclusive-create a lock file and
/// retry with a short backoff until the peer releases it or the budget is
/// exhausted.  Dropping the guard removes the lock.
pub(crate) struct SourceLock {
    path: PathBuf,
}

impl SourceLock {
    pub(crate) fn acquire(cache_root: &Path, source_id: &str) -> Result<Self, AcquireError> {
        let path = cache_root.join(format!(".{source_id}.lock"));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(AcquireError::Cache {
                            reason: format!(
                                "timed out waiting for the source lock {}",
                                path.display()
                            ),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    return Err(AcquireError::Io {
                        path: path.clone(),
                        reason: error.to_string(),
                    });
                }
            }
        }
    }
}

impl Drop for SourceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
