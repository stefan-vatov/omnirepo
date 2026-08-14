use super::{
    AuthorityIdentity, DomainError, FilesystemIdentity, ManagedSectionId, ObjectIdentity,
    RelativePath, RenamePaths,
};
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]

pub enum EntryKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileIdentity {
    filesystem: FilesystemIdentity,
    object: ObjectIdentity,
    kind: EntryKind,
    mode: u32,
}

impl FileIdentity {
    pub fn new(
        filesystem: FilesystemIdentity,
        object: ObjectIdentity,
        kind: EntryKind,
        mode: u32,
    ) -> Result<Self, DomainError> {
        AuthorityIdentity::new(filesystem.clone(), object)?;
        Ok(Self {
            filesystem,
            object,
            kind,
            mode,
        })
    }

    pub fn filesystem(&self) -> &FilesystemIdentity {
        &self.filesystem
    }

    pub fn object(&self) -> &ObjectIdentity {
        &self.object
    }

    pub fn kind(&self) -> EntryKind {
        self.kind
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ManagedOwnership {
    WholeFile,
    Section { id: ManagedSectionId },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedTargetIdentity {
    path: RelativePath,
    ownership: ManagedOwnership,
    observed_file: Option<FileIdentity>,
}

impl ManagedTargetIdentity {
    pub fn whole_file(
        path: RelativePath,
        observed_file: Option<FileIdentity>,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            path,
            ownership: ManagedOwnership::WholeFile,
            observed_file,
        })
    }

    pub fn section(
        path: RelativePath,
        id: ManagedSectionId,
        observed_file: Option<FileIdentity>,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            path,
            ownership: ManagedOwnership::Section { id },
            observed_file,
        })
    }

    pub fn path(&self) -> &RelativePath {
        &self.path
    }

    pub fn ownership(&self) -> &ManagedOwnership {
        &self.ownership
    }

    pub fn observed_file(&self) -> Option<&FileIdentity> {
        self.observed_file.as_ref()
    }

    pub(crate) fn conflicts_with(&self, other: &Self) -> bool {
        if self.path != other.path {
            return false;
        }
        match (&self.ownership, &other.ownership) {
            (ManagedOwnership::WholeFile, _) | (_, ManagedOwnership::WholeFile) => true,
            (ManagedOwnership::Section { id: left }, ManagedOwnership::Section { id: right }) => {
                left == right
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TargetChange {
    Added,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
    ModeChanged,
    LinkChanged,
    Untracked,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DirtyProvenance {
    PreExisting,
    CurrentOperation,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IndexEntry {
    path: RelativePath,
    change: TargetChange,
    provenance: DirtyProvenance,
    rename_from: Option<RelativePath>,
}

impl IndexEntry {
    pub fn new(
        path: RelativePath,
        change: TargetChange,
        provenance: DirtyProvenance,
    ) -> Result<Self, DomainError> {
        if matches!(change, TargetChange::Untracked | TargetChange::Renamed) {
            return Err(DomainError::InvalidChangeShape { change });
        }
        Ok(Self {
            path,
            change,
            provenance,
            rename_from: None,
        })
    }

    pub fn renamed(
        from: RelativePath,
        to: RelativePath,
        provenance: DirtyProvenance,
    ) -> Result<Self, DomainError> {
        let paths = RenamePaths::new(from, to.clone())?;
        Ok(Self {
            path: to,
            change: TargetChange::Renamed,
            provenance,
            rename_from: Some(paths.from().clone()),
        })
    }

    pub fn path(&self) -> &RelativePath {
        &self.path
    }

    pub fn change(&self) -> TargetChange {
        self.change
    }

    pub fn provenance(&self) -> DirtyProvenance {
        self.provenance
    }

    pub fn rename_from(&self) -> Option<&RelativePath> {
        self.rename_from.as_ref()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorktreeEntry {
    path: RelativePath,
    change: TargetChange,
    provenance: DirtyProvenance,
    rename_from: Option<RelativePath>,
}

impl WorktreeEntry {
    pub fn new(
        path: RelativePath,
        change: TargetChange,
        provenance: DirtyProvenance,
    ) -> Result<Self, DomainError> {
        if matches!(change, TargetChange::Renamed) {
            return Err(DomainError::InvalidChangeShape { change });
        }
        Ok(Self {
            path,
            change,
            provenance,
            rename_from: None,
        })
    }

    pub fn renamed(
        from: RelativePath,
        to: RelativePath,
        provenance: DirtyProvenance,
    ) -> Result<Self, DomainError> {
        let paths = RenamePaths::new(from, to.clone())?;
        Ok(Self {
            path: to,
            change: TargetChange::Renamed,
            provenance,
            rename_from: Some(paths.from().clone()),
        })
    }

    pub fn path(&self) -> &RelativePath {
        &self.path
    }

    pub fn change(&self) -> TargetChange {
        self.change
    }

    pub fn provenance(&self) -> DirtyProvenance {
        self.provenance
    }

    pub fn rename_from(&self) -> Option<&RelativePath> {
        self.rename_from.as_ref()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IndexState {
    Clean,
    Entries(Vec<IndexEntry>),
}

impl IndexState {
    pub(crate) fn normalize(mut self) -> Result<Self, DomainError> {
        let Self::Entries(entries) = &mut self else {
            return Ok(self);
        };
        if entries.is_empty() {
            return Err(DomainError::EmptyEntries { field: "index" });
        }
        entries.sort();
        for pair in entries.windows(2) {
            if pair[0].path == pair[1].path {
                return Err(DomainError::DuplicateValue {
                    field: "index path",
                    value: String::from_utf8_lossy(pair[0].path.as_bytes()).into_owned(),
                });
            }
        }
        Ok(self)
    }

    pub fn entries(&self) -> &[IndexEntry] {
        match self {
            Self::Clean => &[],
            Self::Entries(entries) => entries,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WorktreeState {
    Clean,
    Entries(Vec<WorktreeEntry>),
}

impl WorktreeState {
    pub(crate) fn normalize(mut self) -> Result<Self, DomainError> {
        let Self::Entries(entries) = &mut self else {
            return Ok(self);
        };
        if entries.is_empty() {
            return Err(DomainError::EmptyEntries { field: "worktree" });
        }
        entries.sort();
        for pair in entries.windows(2) {
            if pair[0].path == pair[1].path {
                return Err(DomainError::DuplicateValue {
                    field: "worktree path",
                    value: String::from_utf8_lossy(pair[0].path.as_bytes()).into_owned(),
                });
            }
        }
        Ok(self)
    }

    pub fn entries(&self) -> &[WorktreeEntry] {
        match self {
            Self::Clean => &[],
            Self::Entries(entries) => entries,
        }
    }
}
