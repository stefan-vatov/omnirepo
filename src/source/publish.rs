//! Atomic publication of complete snapshots into the immutable store.
//!
//! A fully materialized staging tree moves into
//! `<store-root>/<source-id>/<revision>/` with an atomic same-filesystem
//! rename.  Exactly one complete snapshot becomes visible per identity and
//! revision: a concurrent or repeated publication with the same revision
//! reuses the existing snapshot (losers reuse by policy).  An interrupted
//! publication can never expose a partial authoritative snapshot — staging
//! outside the store is not authoritative, and the rename itself is atomic.
//! Readers pin in-use data by the returned immutable cache path.

#![allow(dead_code)]

use super::snapshot::{
    CacheKey, IdentityError, PublishedSnapshot, RevisionId, SnapshotId, SourceIdentity,
};
use std::{error::Error, fmt, fs, path::Path, path::PathBuf};

/// Publication outcome for one identity/revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishOutcome {
    /// This caller moved the complete staging tree into the store.
    Published(PublishedSnapshot),
    /// The snapshot already exists; the caller reuses it.
    Reused(PublishedSnapshot),
}

/// Typed publication failures; the store is never left partial.
#[derive(Debug)]
pub enum PublishError {
    /// The staging tree is missing, not a directory, or an alias.
    InvalidStaging {
        path: PathBuf,
        reason: String,
    },
    /// The store root is unusable.
    Store {
        path: PathBuf,
        reason: String,
    },
    /// The pre-existing target is not a directory and cannot be reused.
    ConflictingTarget {
        path: PathBuf,
    },
    /// The atomic rename or durability sync failed.
    Io {
        path: PathBuf,
        reason: String,
    },
    Identity(IdentityError),
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStaging { path, reason } => {
                write!(
                    formatter,
                    "invalid snapshot staging {}: {reason}",
                    path.display()
                )
            }
            Self::Store { path, reason } => {
                write!(
                    formatter,
                    "snapshot store {} is unusable: {reason}",
                    path.display()
                )
            }
            Self::ConflictingTarget { path } => write!(
                formatter,
                "snapshot target exists and is not a directory: {}",
                path.display()
            ),
            Self::Io { path, reason } => {
                write!(formatter, "cannot publish {}: {reason}", path.display())
            }
            Self::Identity(error) => write!(formatter, "snapshot identity failure: {error}"),
        }
    }
}
impl Error for PublishError {}

/// Publish a complete staging tree atomically for one source revision.
pub fn publish(
    staging: &Path,
    source: &SourceIdentity,
    revision: &RevisionId,
    store_root: &Path,
) -> Result<PublishOutcome, PublishError> {
    let metadata = fs::symlink_metadata(staging).map_err(|error| PublishError::InvalidStaging {
        path: staging.to_path_buf(),
        reason: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PublishError::InvalidStaging {
            path: staging.to_path_buf(),
            reason: "staging must be a real directory, never an alias".to_owned(),
        });
    }
    if !store_root.is_dir() {
        return Err(PublishError::Store {
            path: store_root.to_path_buf(),
            reason: "store root is not a directory".to_owned(),
        });
    }
    let source_dir = store_root.join(source.id().as_str());
    ensure_source_directory(&source_dir)?;
    let target = source_dir.join(revision.as_str());
    let snapshot_id = SnapshotId::new(format!("{}-{}", source.id().as_str(), revision.as_str()))
        .map_err(PublishError::Identity)?;
    let cache = CacheKey::new(target.display().to_string()).map_err(PublishError::Identity)?;
    let snapshot = PublishedSnapshot::new(source.clone(), revision.clone(), snapshot_id, cache);

    match fs::rename(staging, &target) {
        Ok(()) => {
            sync_directory(&source_dir)?;
            Ok(PublishOutcome::Published(snapshot))
        }
        Err(error) => {
            // The target may already exist: a directory is reused (never
            // overwritten), a non-directory is a typed conflict.  Linux
            // reports ENOTEMPTY/ENOTDIR/EEXIST for these shapes.
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.is_dir() => {
                    let _ = fs::remove_dir_all(staging);
                    Ok(PublishOutcome::Reused(snapshot))
                }
                Ok(_) => Err(PublishError::ConflictingTarget { path: target }),
                Err(_) => Err(PublishError::Io {
                    path: target.clone(),
                    reason: error.to_string(),
                }),
            }
        }
    }
}

fn sync_directory(directory: &Path) -> Result<(), PublishError> {
    let file = fs::File::open(directory).map_err(|error| PublishError::Io {
        path: directory.to_path_buf(),
        reason: error.to_string(),
    })?;
    file.sync_all().map_err(|error| PublishError::Io {
        path: directory.to_path_buf(),
        reason: error.to_string(),
    })
}

/// Ensure the per-source store directory exists as a real directory inside
/// the store root; aliases and non-directories fail closed.
fn ensure_source_directory(directory: &Path) -> Result<(), PublishError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(PublishError::InvalidStaging {
            path: directory.to_path_buf(),
            reason: "store subdirectory is a symlink".to_owned(),
        }),
        Ok(metadata) if !metadata.is_dir() => Err(PublishError::Store {
            path: directory.to_path_buf(),
            reason: "store subdirectory is not a directory".to_owned(),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(directory)
            .map_err(|error| PublishError::Io {
                path: directory.to_path_buf(),
                reason: error.to_string(),
            }),
        Err(error) => Err(PublishError::Io {
            path: directory.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}
