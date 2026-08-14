//! Post-agent delta and residue policy.
//!
//! After a confined repair attempt the destination is snapshotted again
//! and compared against the pre-attempt snapshot.  An unchanged
//! destination is a no-op; a changed managed file is the expected delta;
//! any other new file or directory is residue.  Residue is tolerated
//! only when it is on the explicit allowed list — everything else fails
//! the policy.

#![allow(dead_code)]

#[cfg(test)]
mod repair_delta_tests;

use std::{collections::BTreeMap, error::Error, fmt, fs, path::Path};

/// The delta verdict after one repair attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaVerdict {
    /// The destination is byte-identical to the pre-attempt snapshot.
    NoDelta,
    /// The managed content changed — the expected repair effect.
    ExpectedDelta { changed: Vec<String> },
    /// Files or directories appeared that are not on the allowed list.
    Residue { paths: Vec<String> },
}

/// Snapshot failures.
#[derive(Debug)]
pub enum SnapshotError {
    Root {
        path: std::path::PathBuf,
        reason: String,
    },
    Read {
        path: std::path::PathBuf,
        reason: String,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { path, reason } => {
                write!(
                    formatter,
                    "delta snapshot root failure {}: {reason}",
                    path.display()
                )
            }
            Self::Read { path, reason } => {
                write!(
                    formatter,
                    "delta snapshot read failure {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for SnapshotError {}

/// A directory snapshot: relative path -> content identity witness.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DirSnapshot {
    entries: BTreeMap<String, String>,
}

/// Snapshot a directory: every file with its content identity witness.
pub fn snapshot_dir(root: &Path) -> Result<DirSnapshot, SnapshotError> {
    if !root.is_dir() {
        return Err(SnapshotError::Root {
            path: root.to_path_buf(),
            reason: "not a directory".to_owned(),
        });
    }
    let mut entries = BTreeMap::new();
    collect(root, root, &mut entries)?;
    Ok(DirSnapshot { entries })
}

fn collect(
    root: &Path,
    directory: &Path,
    entries: &mut BTreeMap<String, String>,
) -> Result<(), SnapshotError> {
    let read = fs::read_dir(directory).map_err(|error| SnapshotError::Read {
        path: directory.to_path_buf(),
        reason: error.to_string(),
    })?;
    for entry in read {
        let entry = entry.map_err(|error| SnapshotError::Read {
            path: directory.to_path_buf(),
            reason: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, entries)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| SnapshotError::Read {
                    path: path.clone(),
                    reason: error.to_string(),
                })?;
            let bytes = fs::read(&path).map_err(|error| SnapshotError::Read {
                path: path.clone(),
                reason: error.to_string(),
            })?;
            entries.insert(relative.display().to_string(), identity(&bytes));
        }
    }
    Ok(())
}

fn identity(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}:{hash:x}", bytes.len())
}

/// Classify the post-agent delta against the allowed residue list.
///
/// The allowed list applies to *new* entries only: an entry that existed
/// before with the same identity is never residue.
pub fn classify_post_agent_delta(
    before: &DirSnapshot,
    after: &DirSnapshot,
    allowed_residue: &[String],
) -> DeltaVerdict {
    if before.entries == after.entries {
        return DeltaVerdict::NoDelta;
    }
    let mut changed = Vec::new();
    let mut residue = Vec::new();
    for (path, identity) in &after.entries {
        match before.entries.get(path) {
            Some(before_identity) if before_identity == identity => {}
            Some(_) => changed.push(path.clone()),
            None => {
                if !allowed_residue.contains(path) {
                    residue.push(path.clone());
                }
            }
        }
    }
    if !residue.is_empty() {
        return DeltaVerdict::Residue { paths: residue };
    }
    if changed.is_empty() {
        // Only tolerated additions; a removal is also a change.
        let removed = before
            .entries
            .keys()
            .filter(|path| !after.entries.contains_key(*path))
            .cloned()
            .collect::<Vec<_>>();
        if removed.is_empty() {
            DeltaVerdict::NoDelta
        } else {
            DeltaVerdict::ExpectedDelta { changed: removed }
        }
    } else {
        DeltaVerdict::ExpectedDelta { changed }
    }
}
