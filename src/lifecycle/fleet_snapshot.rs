//! The frozen repository snapshot for one admitted destination.
//!
//! Captures the repository facts, the frozen witnesses (with the
//! base-HEAD), and the managed whole-file/section targets with their
//! observed identities read through the typed destination root.  An
//! absent target is a lawful creation case; a non-regular file at a
//! target fails typed.  The snapshot authorizes exactly the planned
//! replacements.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_snapshot_tests;

use crate::lifecycle::sync_plan::SyncPlan;
use crate::platform::{
    AuthorityRoot, DestinationRepositoryRoot, ObjectClass, ReadOnly, RelativePath,
};
use crate::repository::{
    AuthorityIdentity, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitRepositoryState, HeadState, ManagedTargetIdentity, ObjectIdentity,
    RepositoryFacts, RepositoryId, RepositoryRoot, RepositorySnapshot, RevisionId, capture_state,
};
use std::path::Path;

/// Build the frozen snapshot for one destination.
pub fn build_frozen_snapshot(
    working: &Path,
    plan: &SyncPlan,
) -> Result<RepositorySnapshot, String> {
    let git_state = capture_state(working).map_err(|error| error.to_string())?;
    let root = AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(working)
        .map_err(|error| error.to_string())?;
    let root_identity = root.identity();
    let facts = RepositoryFacts::new(
        RepositoryId::new(plan.destination.as_str()).map_err(|error| error.to_string())?,
        RepositoryRoot::new(
            working.display().to_string(),
            domain_authority(root_identity),
        )
        .map_err(|error| error.to_string())?,
        git_state,
    )
    .map_err(|error| error.to_string())?;
    let base_head = match facts.git() {
        GitRepositoryState::Git(git_facts) => match git_facts.head() {
            HeadState::Attached { commit, .. } | HeadState::Detached { commit } => {
                Some(commit.as_str().to_owned())
            }
            HeadState::Unborn => None,
        },
        GitRepositoryState::NonGit => None,
    };
    let witnesses = FrozenWitnesses::new(
        "authority-1",
        "source-1",
        "catalog-1",
        "configuration-1",
        plan.destination.as_str(),
        Vec::new(),
        base_head
            .as_deref()
            .and_then(|head| RevisionId::new(head.to_owned()).ok()),
    )
    .map_err(|error| error.to_string())?;
    // One frozen target per unique selected destination file: sections
    // sharing one file form one atomic per-file group, so the file is the
    // unit of frozen identity.  Rejected items freeze nothing.
    let mut targets = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for item in &plan.items {
        if !matches!(
            item.decision,
            crate::lifecycle::sync_plan::PlanDecision::Selected { .. }
        ) {
            continue;
        }
        if seen.contains(&item.target.as_str()) {
            continue;
        }
        seen.push(item.target.as_str());
        let relative = RelativePath::parse(&item.target).map_err(|error| error.to_string())?;
        let observed = observe_target_identity(&root, working, &item.target, &relative)?;
        let domain_relative =
            crate::repository::RelativePath::from_bytes(relative.display().as_bytes())
                .map_err(|error| error.to_string())?;
        targets.push(
            ManagedTargetIdentity::whole_file(domain_relative, observed)
                .map_err(|error| error.to_string())?,
        );
    }
    RepositorySnapshot::new(facts, witnesses, targets).map_err(|error| error.to_string())
}

/// Observe one destination target's file identity through the typed
/// read-only root.  Only true absence is a lawful creation target
/// (`None`); every other rejection (aliases, links, mounts, non-regular
/// objects) is a typed failure.
pub(crate) fn observe_target_identity(
    root: &AuthorityRoot<DestinationRepositoryRoot, ReadOnly>,
    working: &Path,
    target: &str,
    relative: &RelativePath,
) -> Result<Option<FileIdentity>, String> {
    match root.resolve_read(relative, ObjectClass::RegularFile) {
        Ok(resolved) => {
            let identity = resolved.identity();
            let filesystem = domain_filesystem(identity.filesystem());
            let object = domain_object(identity.object());
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(working.join(target))
                    .map_err(|error| error.to_string())?
                    .permissions()
                    .mode()
            };
            #[cfg(not(unix))]
            let mode = 0o100644;
            Ok(Some(
                FileIdentity::new(filesystem, object, EntryKind::RegularFile, mode)
                    .map_err(|error| error.to_string())?,
            ))
        }
        Err(crate::platform::PathError::NotFound { .. }) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Convert the platform filesystem identity to the domain type.  The
/// filesystem class is the running host's class (the supported-platform
/// matrix maps one-to-one).
fn domain_filesystem(identity: crate::platform::FilesystemIdentity) -> FilesystemIdentity {
    #[cfg(target_os = "macos")]
    let kind = FilesystemClass::MacOsApfs;
    #[cfg(not(target_os = "macos"))]
    let kind = FilesystemClass::LinuxExtFamily;
    FilesystemIdentity::new(kind, identity.device(), identity.mount_id())
}

/// Convert the platform object identity to the domain type.
fn domain_object(identity: crate::platform::ObjectIdentity) -> ObjectIdentity {
    ObjectIdentity::new(identity.device(), identity.inode())
}

/// Convert the platform authority identity to the domain type.
fn domain_authority(identity: crate::platform::AuthorityIdentity) -> AuthorityIdentity {
    let filesystem = domain_filesystem(identity.filesystem());
    let object = domain_object(identity.object());
    AuthorityIdentity::new(filesystem.clone(), object).expect("authority identity")
}
