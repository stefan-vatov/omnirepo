use super::{
    AuthorityIdentity, CanonicalRepresentation, DomainError, FileIdentity, FrozenWitnesses,
    ManagedTargetIdentity, RelativePath, RenamePaths, RepositoryId, RepositorySnapshot, RevisionId,
    TargetChange,
};
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthorizedChange {
    pub(crate) target: ManagedTargetIdentity,
    pub(crate) change: TargetChange,
    pub(crate) before: Option<FileIdentity>,
    pub(crate) after: Option<FileIdentity>,
    rename_from: Option<RelativePath>,
}

impl AuthorizedChange {
    pub fn new(
        target: ManagedTargetIdentity,
        change: TargetChange,
        before: Option<FileIdentity>,
        after: Option<FileIdentity>,
    ) -> Result<Self, DomainError> {
        let valid = match change {
            TargetChange::Added => before.is_none() && after.is_some(),
            TargetChange::Untracked => false,
            TargetChange::Deleted => before.is_some() && after.is_none(),
            TargetChange::Modified
            | TargetChange::TypeChanged
            | TargetChange::ModeChanged
            | TargetChange::LinkChanged => before.is_some() && after.is_some(),
            TargetChange::Renamed => false,
        };
        if !valid {
            return Err(DomainError::InvalidChangeShape { change });
        }
        if target.observed_file() != before.as_ref() {
            return Err(DomainError::InvalidChangeShape { change });
        }
        Ok(Self {
            target,
            change,
            before,
            after,
            rename_from: None,
        })
    }

    pub fn renamed(
        from: RelativePath,
        target: ManagedTargetIdentity,
        before: Option<FileIdentity>,
        after: Option<FileIdentity>,
    ) -> Result<Self, DomainError> {
        let rename_paths = RenamePaths::new(from, target.path().clone())?;
        if before.is_none() || after.is_none() || target.observed_file() != before.as_ref() {
            return Err(DomainError::InvalidChangeShape {
                change: TargetChange::Renamed,
            });
        }
        Ok(Self {
            target,
            change: TargetChange::Renamed,
            before,
            after,
            rename_from: Some(rename_paths.from().clone()),
        })
    }

    pub fn target(&self) -> &ManagedTargetIdentity {
        &self.target
    }

    pub fn change(&self) -> TargetChange {
        self.change
    }

    pub fn before(&self) -> Option<&FileIdentity> {
        self.before.as_ref()
    }

    pub fn after(&self) -> Option<&FileIdentity> {
        self.after.as_ref()
    }

    pub fn rename_from(&self) -> Option<&RelativePath> {
        self.rename_from.as_ref()
    }

    fn changed_paths(&self) -> Vec<&RelativePath> {
        let mut paths = vec![self.target.path()];
        if let Some(rename_from) = self.rename_from() {
            paths.push(rename_from);
        }
        paths
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthorizedDelta {
    pub(crate) repository_id: RepositoryId,
    pub(crate) authority_identity: AuthorityIdentity,
    pub(crate) snapshot_identity: CanonicalRepresentation,
    pub(crate) witnesses: FrozenWitnesses,
    pub(crate) frozen_targets: Vec<ManagedTargetIdentity>,
    pub(crate) base_head: Option<RevisionId>,
    pub(crate) changes: Vec<AuthorizedChange>,
}

impl AuthorizedDelta {
    pub fn from_snapshot(
        snapshot: &RepositorySnapshot,
        mut changes: Vec<AuthorizedChange>,
    ) -> Result<Self, DomainError> {
        for change in &changes {
            if !snapshot.targets().contains(change.target()) {
                return Err(DomainError::UnauthorizedTarget {
                    path: String::from_utf8_lossy(change.target().path().as_bytes()).into_owned(),
                });
            }
            if let Some(rename_from) = change.rename_from() {
                let source_is_frozen = snapshot.targets().iter().any(|target| {
                    target.path() == rename_from
                        && target.ownership() == change.target().ownership()
                        && target.observed_file() == change.before()
                });
                if !source_is_frozen {
                    return Err(DomainError::UnauthorizedTarget {
                        path: String::from_utf8_lossy(rename_from.as_bytes()).into_owned(),
                    });
                }
            }
        }
        for (index, change) in changes.iter().enumerate() {
            for changed_path in change.changed_paths() {
                for (other_index, other) in changes.iter().enumerate() {
                    if index == other_index {
                        continue;
                    }
                    if other.changed_paths().contains(&changed_path)
                        && (change.rename_from().is_some() || other.rename_from().is_some())
                    {
                        return Err(DomainError::ConflictingTarget {
                            path: String::from_utf8_lossy(changed_path.as_bytes()).into_owned(),
                        });
                    }
                }
            }
        }
        changes.sort();
        for pair in changes.windows(2) {
            if pair[0].target == pair[1].target {
                return Err(DomainError::DuplicateValue {
                    field: "authorized target",
                    value: String::from_utf8_lossy(pair[0].target.path().as_bytes()).into_owned(),
                });
            }
            if pair[0].target.conflicts_with(&pair[1].target) {
                return Err(DomainError::ConflictingTarget {
                    path: String::from_utf8_lossy(pair[0].target.path().as_bytes()).into_owned(),
                });
            }
        }
        Ok(Self {
            repository_id: snapshot.facts().repository_id().clone(),
            authority_identity: snapshot.identity(),
            snapshot_identity: snapshot.canonical_representation(),
            witnesses: snapshot.witnesses().clone(),
            frozen_targets: snapshot.targets().to_vec(),
            base_head: snapshot.witnesses().base_head().cloned(),
            changes,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn snapshot_identity(&self) -> &CanonicalRepresentation {
        &self.snapshot_identity
    }

    pub fn authority_identity(&self) -> &AuthorityIdentity {
        &self.authority_identity
    }

    pub fn witnesses(&self) -> &FrozenWitnesses {
        &self.witnesses
    }

    pub fn frozen_targets(&self) -> &[ManagedTargetIdentity] {
        &self.frozen_targets
    }

    pub fn base_head(&self) -> Option<&RevisionId> {
        self.base_head.as_ref()
    }

    pub fn changes(&self) -> &[AuthorizedChange] {
        &self.changes
    }
}
