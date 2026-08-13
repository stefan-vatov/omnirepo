//! Restart cleanup and transaction evidence for managed replacements.
//!
//! After a crash, owned temporary artifacts are handled deterministically:
//! artifacts matching the exact owned grammar are removed, ambiguous
//! lookalikes fail without any mutation, and non-owned files are never
//! touched.  Every compare/write/publish/cleanup outcome is recorded as a
//! journal evidence event carrying the exact artifact identity, so a later
//! replay can prove what happened at each stage.

#![allow(dead_code)]

use super::journal::{JournalError, JournalHandle};
use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent};
use std::{error::Error, fmt, fs, path::Path, path::PathBuf};

#[cfg(test)]
mod transaction_evidence_tests;

/// The owned temporary marker inside a replacement artifact name.
pub const OWNED_TEMP_MARKER: &str = ".omnirepo-tmp-";

/// Result of one restart cleanup pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupReport {
    pub removed: Vec<PathBuf>,
}

/// Typed restart-cleanup failures.
#[derive(Debug)]
pub enum CleanupError {
    /// The scan directory is unusable.
    Scan { path: PathBuf, reason: String },
    /// An artifact looks owned but does not match the exact grammar; no
    /// mutation happens.
    Ambiguous { path: PathBuf },
    /// An owned artifact could not be removed.
    Remove { path: PathBuf, reason: String },
}

impl fmt::Display for CleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan { path, reason } => {
                write!(formatter, "cannot scan {}: {reason}", path.display())
            }
            Self::Ambiguous { path } => write!(
                formatter,
                "ambiguous artifact {}: refusing to mutate without exact ownership",
                path.display()
            ),
            Self::Remove { path, reason } => {
                write!(
                    formatter,
                    "cannot remove owned artifact {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for CleanupError {}

/// Deterministic restart cleanup of owned temporary artifacts below a
/// directory (non-recursive, bounded to the directory's direct children).
///
/// A file matches the owned grammar when its name starts with `.`, ends with
/// `.tmp`, and contains the exact owned marker with a non-empty operation id
/// and a positive attempt number:
/// `.<target>.omnirepo-tmp-<operation-id>-<attempt>.tmp`.
pub fn restart_cleanup(directory: &Path) -> Result<CleanupReport, CleanupError> {
    let mut removed = Vec::new();
    let entries = fs::read_dir(directory).map_err(|error| CleanupError::Scan {
        path: directory.to_path_buf(),
        reason: error.to_string(),
    })?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| CleanupError::Scan {
            path: directory.to_path_buf(),
            reason: error.to_string(),
        })?;
        if entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
            && entry
                .file_name()
                .to_string_lossy()
                .contains(OWNED_TEMP_MARKER)
        {
            candidates.push(entry.path());
        }
    }
    candidates.sort();
    for candidate in candidates {
        let name = candidate
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !is_owned_temp_name(&name) {
            // Ambiguous artifacts abort the pass before any mutation.
            return Err(CleanupError::Ambiguous { path: candidate });
        }
        fs::remove_file(&candidate).map_err(|error| CleanupError::Remove {
            path: candidate.clone(),
            reason: error.to_string(),
        })?;
        removed.push(candidate);
    }
    Ok(CleanupReport { removed })
}

/// Exact owned-temp grammar:
/// `.<target>.omnirepo-tmp-<operation-id>-<attempt>.tmp` with a non-empty
/// operation id and a positive integer attempt.
fn is_owned_temp_name(name: &str) -> bool {
    let Some(rest) = name.strip_suffix(".tmp") else {
        return false;
    };
    let Some(marker) = rest.find(OWNED_TEMP_MARKER) else {
        return false;
    };
    if marker == 0 || !rest.starts_with('.') {
        return false;
    }
    let tail = &rest[marker + OWNED_TEMP_MARKER.len()..];
    let Some((operation, attempt)) = tail.rsplit_once('-') else {
        return false;
    };
    if operation.is_empty() {
        return false;
    }
    attempt
        .parse::<u32>()
        .map(|value| value > 0)
        .unwrap_or(false)
}

/// Transaction evidence kinds recorded per stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceStage {
    Compare,
    Write,
    Publish,
    Cleanup,
}

impl EvidenceStage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Compare => "compare",
            Self::Write => "write",
            Self::Publish => "publish",
            Self::Cleanup => "cleanup",
        }
    }
}

/// Record one stage outcome as a journal evidence event carrying the exact
/// artifact path and byte count.  The journal acknowledges the checkpoint
/// only after the line is durable.
pub fn record_outcome(
    journal: &JournalHandle,
    stage: EvidenceStage,
    run_id: &str,
    artifact: &Path,
    bytes: u64,
) -> Result<u64, JournalError> {
    let evidence = EvidenceRef::new(EvidenceKind::Process, artifact.display().to_string(), bytes)
        .map_err(JournalError::Invalid)?;
    journal.submit(JournalEvent::Evidence {
        checkpoint: 0,
        run_id: run_id.to_owned(),
        repository_id: None,
        evidence,
        stage: Some(stage.label()),
    })
}
