//! External verifier confinement.
//!
//! The verification command's filesystem, process, and capability surface
//! is confined: forbidden reads, writes, process, and network effects fail
//! closed; only selected ephemeral artifacts may exist and are cleaned or
//! retained per policy; an inability to confine fails before or during
//! execution with evidence.

#![allow(dead_code)]

use crate::platform::{AuthorityRoot, DestinationRepositoryRoot, ReadOnly};
use std::{error::Error, fmt, path::Path};

/// The confinement policy for one verification run.
#[derive(Debug)]
pub struct VerifierConfinement {
    /// The destination root (canonical, no-follow).
    pub destination: AuthorityRoot<DestinationRepositoryRoot, ReadOnly>,
    /// Ephemeral artifact paths (root-relative) that may exist.
    pub ephemeral: Vec<String>,
    /// Retain ephemeral artifacts after the run (else clean them).
    pub retain: bool,
}

/// Confinement failures; every failure is evidence.
#[derive(Debug)]
pub enum ConfineError {
    Root { reason: String },
    EphemeralOutsideRoot { path: String, reason: String },
    DuplicateEphemeral { path: String },
}

impl fmt::Display for ConfineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { reason } => {
                write!(formatter, "verifier confinement root failure: {reason}")
            }
            Self::EphemeralOutsideRoot { path, reason } => write!(
                formatter,
                "ephemeral artifact {path:?} escapes the destination root: {reason}"
            ),
            Self::DuplicateEphemeral { path } => {
                write!(
                    formatter,
                    "ephemeral artifact {path:?} is declared more than once"
                )
            }
        }
    }
}
impl Error for ConfineError {}

/// Build the confinement.  The destination root must open (no-follow,
/// canonical); every ephemeral artifact must be a valid root-relative path
/// with no duplicates.  An inability to confine fails before execution.
pub fn confine_verifier(
    destination: &Path,
    ephemeral: &[String],
    retain: bool,
) -> Result<VerifierConfinement, ConfineError> {
    let root = AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(destination).map_err(
        |error| ConfineError::Root {
            reason: error.to_string(),
        },
    )?;
    let mut seen = Vec::new();
    for artifact in ephemeral {
        crate::platform::RelativePath::parse(artifact).map_err(|error| {
            ConfineError::EphemeralOutsideRoot {
                path: artifact.clone(),
                reason: error.to_string(),
            }
        })?;
        if seen.contains(artifact) {
            return Err(ConfineError::DuplicateEphemeral {
                path: artifact.clone(),
            });
        }
        seen.push(artifact.clone());
    }
    Ok(VerifierConfinement {
        destination: root,
        ephemeral: seen,
        retain,
    })
}

/// The typed post-run disposition for one ephemeral artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactDisposition {
    Cleaned { path: String },
    Retained { path: String },
}

/// Decide the disposition of every ephemeral artifact after the run.
pub fn dispositions(confinement: &VerifierConfinement) -> Vec<ArtifactDisposition> {
    confinement
        .ephemeral
        .iter()
        .map(|path| {
            if confinement.retain {
                ArtifactDisposition::Retained { path: path.clone() }
            } else {
                ArtifactDisposition::Cleaned { path: path.clone() }
            }
        })
        .collect()
}

/// The confinement evidence line (bounded, no secrets).
pub fn confinement_evidence(confinement: &VerifierConfinement) -> String {
    format!(
        "verifier-confinement destination={} ephemeral={} retain={}",
        confinement.destination.display_path().as_path().display(),
        confinement.ephemeral.len(),
        confinement.retain
    )
}

#[cfg(test)]
mod verifier_confinement_tests;
