//! Deterministic side-effect-free replay of run-record journal files.
//!
//! Startup reads only the canonical runs directory, classifies every record
//! as complete or incomplete, and reconstructs the last durable state.  A
//! valid prefix followed by a truncated or corrupt tail replays exactly under
//! policy: the prefix is authoritative, the tail is typed, and an incomplete
//! run can never claim success.

#![allow(dead_code)]

use super::event::{EventError, EventLog, JournalEvent};
use std::{error::Error, fmt, fs, path::Path, path::PathBuf};

#[cfg(test)]
mod replay_tests;

/// Maximum bytes read per record during replay.
pub const MAX_REPLAY_BYTES: u64 = 64 * 1024 * 1024;

/// How the end of a record classifies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TailStatus {
    /// The record ends with a valid terminal state or a clean last line.
    Clean,
    /// The final line is incomplete (no terminating newline or partial JSON).
    Truncated { line: usize },
    /// A complete line is malformed or violates transitions.
    Corrupt { line: usize, reason: String },
    /// A line carries an unsupported journal version.
    UnsupportedVersion { line: usize, version: u64 },
}

/// Reconstructed replay of one record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Replay {
    pub events: Vec<JournalEvent>,
    pub tail: TailStatus,
    pub complete: bool,
}

/// Replay a run record.  Side-effect free and deterministic: the file is
/// read once, lines are parsed in order, transitions are validated, and the
/// first unparsable or transition-invalid line classifies the tail.
pub fn replay(path: &Path) -> Result<Replay, ReplayError> {
    let metadata = fs::metadata(path).map_err(|error| ReplayError::Read {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(ReplayError::Read {
            path: path.to_path_buf(),
            reason: "record is not a regular file".to_owned(),
        });
    }
    if metadata.len() > MAX_REPLAY_BYTES {
        return Err(ReplayError::Read {
            path: path.to_path_buf(),
            reason: "record exceeds the replay size bound".to_owned(),
        });
    }
    let content = fs::read_to_string(path).map_err(|error| ReplayError::Read {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let mut log = EventLog::new();
    let mut events = Vec::new();
    let mut tail = TailStatus::Clean;
    // Split on newlines manually: a final chunk without a terminating newline
    // is a truncated tail (the writer was interrupted mid-line), never parsed
    // and never authoritative.  A complete line that fails to parse is a
    // typed corrupt tail.
    let chunks: Vec<&str> = content.split('\n').collect();
    let ends_with_newline = content.ends_with('\n');
    for (index, chunk) in chunks.iter().enumerate() {
        let line_number = index + 1;
        let is_last_chunk = index + 1 == chunks.len();
        if chunk.trim().is_empty() {
            if is_last_chunk {
                break; // trailing newline
            }
            tail = TailStatus::Corrupt {
                line: line_number,
                reason: "blank journal line".to_owned(),
            };
            break;
        }
        if is_last_chunk && !ends_with_newline {
            tail = TailStatus::Truncated { line: line_number };
            break;
        }
        match JournalEvent::parse(chunk) {
            Ok(event) => {
                if let Err(error) = log.record(&event) {
                    tail = TailStatus::Corrupt {
                        line: line_number,
                        reason: error.to_string(),
                    };
                    break;
                }
                events.push(event);
            }
            Err(EventError::UnknownVersion(version)) => {
                tail = TailStatus::UnsupportedVersion {
                    line: line_number,
                    version,
                };
                break;
            }
            Err(error) => {
                tail = TailStatus::Corrupt {
                    line: line_number,
                    reason: error.to_string(),
                };
                break;
            }
        }
    }
    Ok(Replay {
        complete: tail == TailStatus::Clean && log.is_terminal(),
        tail,
        events,
    })
}

/// List incomplete run records below a canonical runs directory.
///
/// Only direct `*.log` children are considered; unrelated files and nested
/// directories are never scanned.
pub fn discover_incomplete(runs_directory: &Path) -> Result<Vec<PathBuf>, ReplayError> {
    let mut incomplete = Vec::new();
    let entries = fs::read_dir(runs_directory).map_err(|error| ReplayError::Read {
        path: runs_directory.to_path_buf(),
        reason: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| ReplayError::Read {
            path: runs_directory.to_path_buf(),
            reason: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("log") {
            continue;
        }
        let replay = replay(&path)?;
        if !replay.complete {
            incomplete.push(path);
        }
    }
    incomplete.sort();
    Ok(incomplete)
}

#[derive(Debug)]
pub enum ReplayError {
    Read { path: PathBuf, reason: String },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, reason } => {
                write!(formatter, "cannot replay {}: {reason}", path.display())
            }
        }
    }
}
impl Error for ReplayError {}
