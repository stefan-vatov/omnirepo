//! Destination-only agent filesystem and process confinement.
//!
//! Repair agents execute with the destination repository as their working
//! directory, a minimal sanitized environment (no ambient credential
//! variables, no repository-controlled hooks), a PATH limited to the
//! resolved adapters, and a bounded process environment.  Every configured
//! path must stay inside the destination root.

#![allow(dead_code)]

use crate::platform::{AgentWorkingDirectoryRoot, AuthorityRoot, ReadOnly};
use std::{error::Error, fmt, path::PathBuf};

#[cfg(test)]
mod agent_confinement_tests;

/// The confined agent execution context.
#[derive(Clone, Debug)]
pub struct AgentConfinement {
    pub workdir: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Confinement failures.
#[derive(Debug)]
pub enum ConfinementError {
    /// A configured path escapes the destination root.
    EscapesDestination { path: PathBuf, root: PathBuf },
    /// The destination root is unusable.
    Root { path: PathBuf, reason: String },
}

impl fmt::Display for ConfinementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EscapesDestination { path, root } => write!(
                formatter,
                "agent path {} escapes the destination root {}",
                path.display(),
                root.display()
            ),
            Self::Root { path, reason } => {
                write!(
                    formatter,
                    "agent confinement root {} is unusable: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for ConfinementError {}

/// Build the confinement for one destination repository.
///
/// The destination is accepted only through the typed agent working
/// directory root (no-follow authority): the root is already canonical and
/// alias-free.  `adapter_paths` are the resolved adapter executable
/// directories (already machine-validated); `extra_paths` are additional
/// configured directories that must be checked for containment.
pub fn confine(
    destination: &AuthorityRoot<AgentWorkingDirectoryRoot, ReadOnly>,
    adapter_paths: &[PathBuf],
    extra_paths: &[PathBuf],
) -> Result<AgentConfinement, ConfinementError> {
    let canonical_destination = destination.display_path().as_path().to_path_buf();
    if !canonical_destination.is_dir() {
        return Err(ConfinementError::Root {
            path: canonical_destination.clone(),
            reason: "destination is not a directory".to_owned(),
        });
    }
    let mut env = Vec::new();
    // A minimal environment: the destination is HOME, the temp dir is below
    // the destination, and PATH is limited to the resolved adapters.
    env.push((
        "HOME".to_owned(),
        canonical_destination.display().to_string(),
    ));
    env.push((
        "TMPDIR".to_owned(),
        canonical_destination
            .join(".omnirepo-tmp")
            .display()
            .to_string(),
    ));
    let mut path_value = String::new();
    for directory in adapter_paths {
        if !directory.starts_with(&canonical_destination) {
            // Adapter executables may live outside the destination (machine
            // toolchain); they are still allowed as read-only PATH entries.
        }
        if !path_value.is_empty() {
            path_value.push(':');
        }
        path_value.push_str(&directory.display().to_string());
    }
    env.push(("PATH".to_owned(), path_value));
    // Every extra configured path must stay inside the destination.
    for path in extra_paths {
        let canonical = path
            .canonicalize()
            .map_err(|error| ConfinementError::Root {
                path: path.clone(),
                reason: error.to_string(),
            })?;
        if !canonical.starts_with(&canonical_destination) {
            return Err(ConfinementError::EscapesDestination {
                path: canonical,
                root: canonical_destination.clone(),
            });
        }
    }
    Ok(AgentConfinement {
        workdir: canonical_destination,
        env,
    })
}
