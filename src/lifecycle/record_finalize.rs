//! Atomic record finalization and canonical discovery identity.
//!
//! A completed record replays as terminal; an interrupted or
//! finalization-failed record stays distinguishable and discoverable
//! (discover_incomplete finds it).  Finalization is idempotent and never
//! emits a false path or reference: the marker is the canonical Terminal
//! event line appended and synced atomically.

#![allow(dead_code)]

use crate::lifecycle::event::{JournalEvent, Outcome};
use crate::lifecycle::journal::JournalError;
use crate::lifecycle::run_record::RunRecord;
use std::{error::Error, fmt, fs, path::Path};

/// Finalization outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalizeOutcome {
    /// The terminal marker was appended.
    Finalized,
    /// The record was already terminal; nothing changed.
    AlreadyFinalized,
}

/// Finalization failures.
#[derive(Debug)]
pub enum FinalizeError {
    Read { path: String, reason: String },
    Append { path: String, reason: String },
    Sync { path: String, reason: String },
    Invalid { reason: String },
}

impl fmt::Display for FinalizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, reason } => {
                write!(formatter, "cannot read record {path}: {reason}")
            }
            Self::Append { path, reason } => {
                write!(formatter, "cannot finalize record {path}: {reason}")
            }
            Self::Sync { path, reason } => {
                write!(formatter, "cannot sync record {path}: {reason}")
            }
            Self::Invalid { reason } => write!(formatter, "finalization refused: {reason}"),
        }
    }
}
impl Error for FinalizeError {}

/// The terminal marker line, canonical and versioned.
pub fn terminal_marker(run_id: &str) -> String {
    JournalEvent::Terminal {
        checkpoint: 0,
        run_id: run_id.to_owned(),
        outcome: Outcome::Success,
    }
    .render()
}

/// Finalize a record atomically: append the terminal marker with a
/// bounded append (never truncate, never rewrite) and sync.  Idempotent:
/// an already-terminal record is reported and left untouched.
pub fn finalize_record(record: &RunRecord, run_id: &str) -> Result<FinalizeOutcome, FinalizeError> {
    finalize_path(record.path(), run_id)
}

/// Finalize by canonical path (crash-recovery finalizer).
pub fn finalize_path(path: &Path, run_id: &str) -> Result<FinalizeOutcome, FinalizeError> {
    let content = fs::read_to_string(path).map_err(|error| FinalizeError::Read {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    if content
        .lines()
        .any(|line| line.contains("\"type\":\"terminal\""))
    {
        return Ok(FinalizeOutcome::AlreadyFinalized);
    }
    let marker = terminal_marker(run_id);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| FinalizeError::Append {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    use std::io::Write;
    file.write_all(marker.as_bytes())
        .map_err(|error| FinalizeError::Append {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    crate::platform::sync_file(&file, &path.display().to_string()).map_err(|error| {
        FinalizeError::Sync {
            path: path.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    Ok(FinalizeOutcome::Finalized)
}

/// Classify a journal failure for finalization evidence.
pub fn finalization_failure_label(error: &JournalError) -> &'static str {
    match error {
        JournalError::Write(_) => "write",
        JournalError::Invalid(_) => "invalid",
        JournalError::Poisoned => "poisoned",
    }
}

#[cfg(test)]
mod record_finalize_tests;
