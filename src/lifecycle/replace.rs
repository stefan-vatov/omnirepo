//! Containment-aware old-or-complete-new replacement executor.
//!
//! Given a prepared transaction plan and exact authoritative bytes, the
//! executor creates a temporary sibling through the platform authority
//! (no-follow, exclusive, mode-applied), writes and syncs the exact bytes,
//! then publishes atomically with an old-or-complete-new contract: the old
//! target stays authoritative until the atomic rename, and an injected
//! failure at any stage exposes only allowed old/new/residue states with the
//! owned temporary cleaned up.

#![allow(dead_code)]

use crate::managed_content::{ParentDirectories, TempCandidate, TransactionPlan};
use crate::platform::{
    DestinationRepositoryRoot, MutationIntent, PathError, RelativePath, open_mutation_root,
    sync_directory, sync_file,
};
use std::{error::Error, fmt, io::Write, path::Path, path::PathBuf};

#[cfg(test)]
mod replace_tests;

/// The decided metadata mode for the published replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecidedMode {
    /// A new file: mode 0644 subject to the process umask.
    Create,
    /// An existing file: the exact preserved mode bits.
    Preserve(u32),
}

/// The fixed creation mode for new managed files.
const CREATE_MODE: u32 = 0o644;

/// Requested replacement of one managed target.
#[derive(Clone, Debug)]
pub struct ReplaceRequest {
    pub plan: TransactionPlan,
    pub content: Vec<u8>,
    /// Decided metadata mode applied to the temporary before publish.
    pub mode: DecidedMode,
}

impl ReplaceRequest {
    pub fn new(plan: TransactionPlan, content: impl Into<Vec<u8>>, mode: DecidedMode) -> Self {
        Self {
            plan,
            content: content.into(),
            mode,
        }
    }
}

/// Typed replacement failures; the old target is always preserved.
#[derive(Debug)]
pub enum ReplaceError {
    Authority { reason: String },
    Resolve { reason: String },
    CreateParent { path: PathBuf, reason: String },
    CreateTemp { reason: String },
    Write { reason: String },
    Metadata { reason: String },
    Sync { reason: String },
    Publish { reason: String },
    Cleanup { path: PathBuf, reason: String },
}

impl fmt::Display for ReplaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority { reason } => {
                write!(formatter, "replacement authority failure: {reason}")
            }
            Self::Resolve { reason } => {
                write!(formatter, "cannot resolve replacement target: {reason}")
            }
            Self::CreateParent { path, reason } => write!(
                formatter,
                "cannot create replacement parent {}: {reason}",
                path.display()
            ),
            Self::CreateTemp { reason } => {
                write!(formatter, "cannot create replacement temporary: {reason}")
            }
            Self::Write { reason } => write!(formatter, "cannot write replacement bytes: {reason}"),
            Self::Metadata { reason } => {
                write!(formatter, "cannot preserve replacement metadata: {reason}")
            }
            Self::Sync { reason } => write!(formatter, "cannot sync replacement bytes: {reason}"),
            Self::Publish { reason } => write!(formatter, "cannot publish replacement: {reason}"),
            Self::Cleanup { path, reason } => write!(
                formatter,
                "cannot clean replacement temporary {}: {reason}",
                path.display()
            ),
        }
    }
}
impl Error for ReplaceError {}

/// Execute the prepared replacement under the platform authority.
///
/// The temporary is created as an exclusive sibling of the target (same
/// directory, same filesystem), written and synced, then atomically renamed
/// over the target; the parent directory is synced afterwards.  Any failure
/// before the rename leaves the old target byte-identical and removes the
/// owned temporary.
pub fn replace(root: &Path, request: &ReplaceRequest) -> Result<(), ReplaceError> {
    let authority = open_mutation_root::<DestinationRepositoryRoot>(root).map_err(|error| {
        ReplaceError::Authority {
            reason: error.to_string(),
        }
    })?;
    let target_relative = RelativePath::parse(&plan_display(&request.plan)).map_err(|error| {
        ReplaceError::Resolve {
            reason: error.to_string(),
        }
    })?;
    // Replace revalidates an existing target; a missing target is a create
    // candidate resolved exclusively (old-or-complete-new covers absence).
    let target = match authority.resolve_mutation(&target_relative, MutationIntent::Replace) {
        Ok(target) => target,
        Err(PathError::NotFound { .. }) => authority
            .resolve_mutation(&target_relative, MutationIntent::CreateExclusive)
            .map_err(|error| ReplaceError::Resolve {
                reason: error.to_string(),
            })?,
        Err(error) => {
            return Err(ReplaceError::Resolve {
                reason: error.to_string(),
            });
        }
    };
    let parent = target
        .clone_parent()
        .map_err(|error| ReplaceError::Resolve {
            reason: error.to_string(),
        })?;

    let temp_relative = temp_path(&request.plan)?;
    let temp = authority
        .resolve_mutation(&temp_relative, MutationIntent::CreateExclusive)
        .map_err(|error| ReplaceError::CreateTemp {
            reason: error.to_string(),
        })?;
    let create_mode = match request.mode {
        DecidedMode::Create => CREATE_MODE,
        DecidedMode::Preserve(mode) => mode & 0o7777,
    };
    let mut file = temp
        .create_exclusive_with_mode(create_mode)
        .map_err(|error| ReplaceError::CreateTemp {
            reason: error.to_string(),
        })?;
    let write_result = (|| -> Result<(), ReplaceError> {
        // A preserved mode is exact: the umask applied at creation, so the
        // preserved bits are restored explicitly before publish.
        if let DecidedMode::Preserve(mode) = request.mode {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))
                    .map_err(|error| ReplaceError::Metadata {
                        reason: error.to_string(),
                    })?;
            }
        }
        file.write_all(&request.content)
            .map_err(|error| ReplaceError::Write {
                reason: error.to_string(),
            })?;
        sync_file(
            &file,
            &root.join(temp_relative.display()).display().to_string(),
        )
        .map_err(|error| ReplaceError::Sync {
            reason: error.to_string(),
        })?;
        drop(file);
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(root.join(temp_relative.display()));
        return Err(error);
    }

    let target_absolute = root.join(target_relative.display());
    let temp_absolute = root.join(temp_relative.display());
    if let Err(error) = std::fs::rename(&temp_absolute, &target_absolute) {
        let _ = std::fs::remove_file(&temp_absolute);
        return Err(ReplaceError::Publish {
            reason: error.to_string(),
        });
    }
    sync_directory(&parent, &root.display().to_string()).map_err(|error| ReplaceError::Sync {
        reason: error.to_string(),
    })?;
    Ok(())
}

/// The plan's target as a relative path string.
fn plan_display(plan: &TransactionPlan) -> String {
    plan.target().display().to_string()
}

/// The owned temporary sibling path for one target and operation, as a
/// relative display string.  The operation id is reduced to a single
/// path component (every character outside `[A-Za-z0-9._-]` becomes
/// `-`) so the temporary always stays a sibling of the target.
pub fn owned_temp_display(target: &str, operation_id: &str) -> String {
    let (parent, file_name) = match target.rfind('/') {
        Some(index) => (&target[..index], &target[index + 1..]),
        None => ("", target),
    };
    let operation_id: String = operation_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let temp_name = format!(".{file_name}.omnirepo-tmp-{operation_id}-1.tmp");
    if parent.is_empty() {
        temp_name
    } else {
        format!("{parent}/{temp_name}")
    }
}

/// The owned temporary name: a sibling of the target, keyed by the operation
/// identity and a fixed attempt, validated through the domain's candidate
/// rules.
fn temp_path(plan: &TransactionPlan) -> Result<RelativePath, ReplaceError> {
    let target = plan.target().display().to_string();
    let combined = owned_temp_display(&target, plan.operation_id());
    let candidate = TempCandidate::new(PathBuf::from(&combined), 1).map_err(|error| {
        ReplaceError::CreateTemp {
            reason: error.to_string(),
        }
    })?;
    RelativePath::parse(&candidate.path().display().to_string()).map_err(|error| {
        ReplaceError::CreateTemp {
            reason: error.to_string(),
        }
    })
}

/// Decide the replacement metadata and publish `bytes` at `target`
/// atomically through the executor: an existing regular file keeps its
/// exact mode; an absent target is created with mode 0644 subject to the
/// process umask, creating safe contained parent directories first.  A
/// failure removes only the empty parents this operation created.
pub fn replace_bytes_atomically(
    root: &Path,
    target: &str,
    operation_id: &str,
    bytes: &[u8],
) -> Result<(), ReplaceError> {
    let mode = match std::fs::symlink_metadata(root.join(target)) {
        Ok(metadata) if metadata.is_file() => {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode() & 0o7777;
                if mode & 0o222 == 0 {
                    return Err(ReplaceError::Metadata {
                        reason: format!("managed target {target} is read-only"),
                    });
                }
                DecidedMode::Preserve(mode)
            };
            #[cfg(not(unix))]
            let mode = DecidedMode::Create;
            mode
        }
        // A non-regular object reaches the executor's typed rejection.
        Ok(_) => DecidedMode::Create,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DecidedMode::Create,
        Err(error) => {
            return Err(ReplaceError::Resolve {
                reason: error.to_string(),
            });
        }
    };
    replace_bytes_atomically_with_decided_mode(root, target, operation_id, bytes, mode)
}

/// Publish exact bytes with an already frozen mode. Verification restoration
/// uses this path so a verifier cannot make its metadata authoritative.
pub(crate) fn replace_bytes_atomically_with_mode(
    root: &Path,
    target: &str,
    operation_id: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<(), ReplaceError> {
    replace_bytes_atomically_with_decided_mode(
        root,
        target,
        operation_id,
        bytes,
        DecidedMode::Preserve(mode),
    )
}

fn replace_bytes_atomically_with_decided_mode(
    root: &Path,
    target: &str,
    operation_id: &str,
    bytes: &[u8],
    mode: DecidedMode,
) -> Result<(), ReplaceError> {
    // Safe contained parent creation: the target path is validated by the
    // plan contract below before any created directory is used, and the
    // ancestors are created inside the root only.
    let mut created_parents: Vec<PathBuf> = Vec::new();
    let components: Vec<&str> = target.split('/').collect();
    let mut prefix = PathBuf::new();
    for component in &components[..components.len().saturating_sub(1)] {
        prefix.push(component);
        if !root.join(&prefix).exists() {
            created_parents.push(prefix.clone());
        }
    }
    let plan = TransactionPlan::new(
        operation_id,
        PathBuf::from(target),
        if created_parents.is_empty() {
            ParentDirectories::existing()
        } else {
            ParentDirectories::created(created_parents.clone())
        },
    )
    .map_err(|error| ReplaceError::Resolve {
        reason: error.to_string(),
    })?;
    let remove_created = |created: &[PathBuf]| {
        // After failure, remove only the empty parents this operation
        // created (deepest first); a parent that gained content stays.
        for parent in created.iter().rev() {
            let _ = std::fs::remove_dir(root.join(parent));
        }
    };
    for (index, parent) in created_parents.iter().enumerate() {
        if let Err(error) = std::fs::create_dir(root.join(parent)) {
            remove_created(&created_parents[..index]);
            return Err(ReplaceError::CreateParent {
                path: parent.clone(),
                reason: error.to_string(),
            });
        }
    }
    let result = replace(root, &ReplaceRequest::new(plan, bytes.to_vec(), mode));
    if result.is_err() {
        remove_created(&created_parents);
    }
    result
}
