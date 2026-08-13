//! Deterministic configured agent-adapter resolution.
//!
//! The machine-declared repair priority (an ordered AgentKind list) resolves
//! to adapter entries with a stable order.  Each entry carries the resolved
//! executable path and a replacement-detection identity (size + mtime of the
//! canonical file).  An absent executable excludes that entry; an empty
//! priority list yields no adapters (repair unavailable, lawful); a
//! non-empty list with every executable absent is exhausted and fails.
//! Repository policy cannot reach this resolution: the input is the
//! machine-validated priority only.

#![allow(dead_code)]

use crate::configuration::AgentKind;
use std::{error::Error, fmt, fs, path::Path, path::PathBuf, time::SystemTime};

#[cfg(test)]
mod adapters_tests;

/// One resolved adapter entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterResolution {
    pub kind: AgentKind,
    pub executable: PathBuf,
    /// Replacement-detection identity: canonical path, size, mtime.
    pub identity: String,
}

/// Typed resolution outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterOutcome {
    /// The ordered resolved adapters.
    Resolved(Vec<AdapterResolution>),
    /// The priority list is empty: repair is unavailable by policy.
    NoneConfigured,
    /// Every listed adapter executable is absent.
    Exhausted { missing: Vec<AgentKind> },
}

/// Resolution failures beyond the policy outcomes.
#[derive(Debug)]
pub enum AdapterError {
    Path { reason: String },
    Io { path: PathBuf, reason: String },
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { reason } => write!(formatter, "adapter path resolution failed: {reason}"),
            Self::Io { path, reason } => {
                write!(
                    formatter,
                    "adapter identity failed for {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for AdapterError {}

/// The executable name for each configured agent kind.
pub fn executable_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Codex => "codex",
        AgentKind::ClaudeCode => "claude",
        AgentKind::Pi => "pi",
    }
}

/// Resolve the ordered adapters from the machine-declared priority list.
///
/// The search is deterministic: the machine-configured adapter paths are
/// probed in order, then PATH (a stable first-match).  Only the machine
/// priority reaches this function; repository policy cannot redefine it.
pub fn resolve_adapters(
    priority: &[AgentKind],
    configured_paths: &[PathBuf],
) -> Result<AdapterOutcome, AdapterError> {
    if priority.is_empty() {
        return Ok(AdapterOutcome::NoneConfigured);
    }
    let mut resolved = Vec::new();
    let mut missing = Vec::new();
    for kind in priority {
        if let Some(executable) = locate(kind, configured_paths)? {
            resolved.push(AdapterResolution {
                kind: *kind,
                identity: executable_identity(&executable)?,
                executable,
            });
        } else {
            missing.push(*kind);
        }
    }
    if resolved.is_empty() {
        Ok(AdapterOutcome::Exhausted { missing })
    } else {
        Ok(AdapterOutcome::Resolved(resolved))
    }
}

fn locate(kind: &AgentKind, configured_paths: &[PathBuf]) -> Result<Option<PathBuf>, AdapterError> {
    let name = executable_name(*kind);
    for directory in configured_paths {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
    }
    let Some(path_value) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    for directory in std::env::split_paths(&path_value) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
    }
    let _ = Path::new(name);
    Ok(None)
}

/// Replacement-detection identity: canonical path plus size and mtime.  A
/// replaced executable changes the identity.
fn executable_identity(executable: &Path) -> Result<String, AdapterError> {
    let canonical = fs::canonicalize(executable).map_err(|error| AdapterError::Io {
        path: executable.to_path_buf(),
        reason: error.to_string(),
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| AdapterError::Io {
        path: executable.to_path_buf(),
        reason: error.to_string(),
    })?;
    let mtime: SystemTime = metadata.modified().map_err(|error| AdapterError::Io {
        path: executable.to_path_buf(),
        reason: error.to_string(),
    })?;
    Ok(format!(
        "{}:{}:{}",
        canonical.display(),
        metadata.len(),
        mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ))
}
