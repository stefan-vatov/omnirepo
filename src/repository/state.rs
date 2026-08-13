//! Pure repository-state values shared by planning, verification, repair, Git,
//! journaling, and recovery.
//!
//! This module intentionally has no filesystem, Git, process, network, or
//! clock effects.  It records observed facts, frozen operation witnesses, and
//! the exact current-run delta as immutable, deterministically ordered values.

use std::{error::Error, fmt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    EmptyValue {
        field: &'static str,
    },
    ControlCharacter {
        field: &'static str,
    },
    InvalidAbsoluteRoot {
        value: String,
    },
    InvalidRelativePath {
        value: String,
    },
    InvalidManagedSectionId {
        value: String,
    },
    InvalidRenamePaths {
        from: String,
        to: String,
    },
    AuthorityDeviceMismatch {
        filesystem_device: u64,
        object_device: u64,
    },
    DuplicateValue {
        field: &'static str,
        value: String,
    },
    EmptyEntries {
        field: &'static str,
    },
    InvalidChangeShape {
        change: TargetChange,
    },
    ConflictingTarget {
        path: String,
    },
    UnauthorizedTarget {
        path: String,
    },
    InvalidCausation {
        relation: CausationRelation,
        basis: CausationBasis,
    },
    InvalidProofBinding {
        field: &'static str,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue { field } => write!(formatter, "{field} must not be empty"),
            Self::ControlCharacter { field } => {
                write!(formatter, "{field} must not contain control characters")
            }
            Self::InvalidAbsoluteRoot { value } => {
                write!(formatter, "invalid absolute repository root {value:?}")
            }
            Self::InvalidRelativePath { value } => {
                write!(formatter, "invalid relative repository path {value:?}")
            }
            Self::InvalidManagedSectionId { value } => {
                write!(formatter, "invalid managed section ID {value:?}")
            }
            Self::InvalidRenamePaths { from, to } => {
                write!(
                    formatter,
                    "rename source and destination must differ: {from:?} -> {to:?}"
                )
            }
            Self::AuthorityDeviceMismatch {
                filesystem_device,
                object_device,
            } => write!(
                formatter,
                "authority filesystem/object device mismatch: filesystem={filesystem_device}, object={object_device}"
            ),
            Self::DuplicateValue { field, value } => {
                write!(formatter, "duplicate {field} value {value:?}")
            }
            Self::EmptyEntries { field } => write!(formatter, "{field} entries must not be empty"),
            Self::InvalidChangeShape { change } => {
                write!(formatter, "invalid before/after shape for {change:?}")
            }
            Self::ConflictingTarget { path } => {
                write!(formatter, "conflicting managed target scope at {path:?}")
            }
            Self::UnauthorizedTarget { path } => {
                write!(
                    formatter,
                    "authorized delta target is outside the frozen snapshot: {path:?}"
                )
            }
            Self::InvalidCausation { relation, basis } => {
                write!(
                    formatter,
                    "invalid causation relation {relation:?} with basis {basis:?}"
                )
            }
            Self::InvalidProofBinding { field } => {
                write!(formatter, "causation proof is not bound to {field}")
            }
        }
    }
}

impl Error for DomainError {}

fn validate_text(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    if value.chars().any(char::is_control) {
        return Err(DomainError::ControlCharacter { field });
    }
    Ok(())
}

macro_rules! text_value {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                validate_text(&value, $field)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

text_value!(RepositoryId, "repository ID");
text_value!(RevisionId, "revision ID");
text_value!(RefName, "ref name");
text_value!(CheckWitness, "check witness");

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedSectionId(String);

impl ManagedSectionId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty()
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
        {
            return Err(DomainError::InvalidManagedSectionId { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositoryRoot {
    path: String,
    authority: AuthorityIdentity,
}

impl RepositoryRoot {
    pub fn new(
        value: impl Into<String>,
        authority: AuthorityIdentity,
    ) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty()
            || !value.starts_with('/')
            || value.as_bytes().contains(&0)
            || value.contains('\\')
            || (value.len() > 1 && value.ends_with('/'))
        {
            return Err(DomainError::InvalidAbsoluteRoot { value });
        }

        let mut components = value.split('/');
        let _root = components.next();
        if components.any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(DomainError::InvalidAbsoluteRoot { value });
        }
        Ok(Self {
            path: value,
            authority,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.path
    }

    pub fn authority(&self) -> &AuthorityIdentity {
        &self.authority
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RelativePath {
    bytes: Vec<u8>,
}

impl RelativePath {
    pub fn new(value: impl AsRef<str>) -> Result<Self, DomainError> {
        Self::from_bytes(value.as_ref().as_bytes())
    }

    pub fn from_bytes(value: impl AsRef<[u8]>) -> Result<Self, DomainError> {
        let value = value.as_ref();
        if value.is_empty() || value[0] == b'/' || value.contains(&0) {
            return Err(DomainError::InvalidRelativePath {
                value: String::from_utf8_lossy(value).into_owned(),
            });
        }

        let mut components: Vec<&[u8]> = Vec::new();
        for component in value.split(|byte| *byte == b'/') {
            if component.is_empty() || component == b"." {
                continue;
            }
            if component == b".." {
                return Err(DomainError::InvalidRelativePath {
                    value: String::from_utf8_lossy(value).into_owned(),
                });
            }
            components.push(component);
        }
        if components.is_empty() {
            return Err(DomainError::InvalidRelativePath {
                value: String::from_utf8_lossy(value).into_owned(),
            });
        }

        let mut normalized = Vec::new();
        for (index, component) in components.iter().enumerate() {
            if index != 0 {
                normalized.push(b'/');
            }
            normalized.extend_from_slice(component);
        }
        Ok(Self { bytes: normalized })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn components(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes.split(|byte| *byte == b'/')
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenamePaths {
    from: RelativePath,
    to: RelativePath,
}

impl RenamePaths {
    pub fn new(from: RelativePath, to: RelativePath) -> Result<Self, DomainError> {
        if from == to {
            return Err(DomainError::InvalidRenamePaths {
                from: String::from_utf8_lossy(from.as_bytes()).into_owned(),
                to: String::from_utf8_lossy(to.as_bytes()).into_owned(),
            });
        }
        Ok(Self { from, to })
    }

    pub fn from(&self) -> &RelativePath {
        &self.from
    }

    pub fn to(&self) -> &RelativePath {
        &self.to
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnsupportedFilesystemName(String);

impl UnsupportedFilesystemName {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_text(&value, "unsupported filesystem name")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FilesystemClass {
    LinuxExtFamily,
    MacOsApfs,
    Other(UnsupportedFilesystemName),
}

impl FilesystemClass {
    pub fn other(value: impl Into<String>) -> Result<Self, DomainError> {
        Ok(Self::Other(UnsupportedFilesystemName::new(value)?))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FilesystemIdentity {
    class: FilesystemClass,
    device: u64,
    mount_id: u64,
}

impl FilesystemIdentity {
    pub const fn new(class: FilesystemClass, device: u64, mount_id: u64) -> Self {
        Self {
            class,
            device,
            mount_id,
        }
    }

    pub fn class(&self) -> &FilesystemClass {
        &self.class
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn mount_id(&self) -> u64 {
        self.mount_id
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectIdentity {
    device: u64,
    inode: u64,
}

impl ObjectIdentity {
    pub const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn inode(&self) -> u64 {
        self.inode
    }
}

/// The collision and containment identity for an authority root.
///
/// A lexical path is only a display value.  The filesystem and root-object
/// identities are the authority-bearing values used for equality and scope.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthorityIdentity {
    filesystem: FilesystemIdentity,
    object: ObjectIdentity,
}

impl AuthorityIdentity {
    pub fn new(
        filesystem: FilesystemIdentity,
        object: ObjectIdentity,
    ) -> Result<Self, DomainError> {
        if filesystem.device() != object.device() {
            return Err(DomainError::AuthorityDeviceMismatch {
                filesystem_device: filesystem.device(),
                object_device: object.device(),
            });
        }
        Ok(Self { filesystem, object })
    }

    pub fn filesystem(&self) -> &FilesystemIdentity {
        &self.filesystem
    }

    pub fn object(&self) -> ObjectIdentity {
        self.object
    }
}

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

    fn conflicts_with(&self, other: &Self) -> bool {
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
    fn normalize(mut self) -> Result<Self, DomainError> {
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
    fn normalize(mut self) -> Result<Self, DomainError> {
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HeadState {
    Unborn,
    Detached { commit: RevisionId },
    Attached { branch: RefName, commit: RevisionId },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UpstreamState {
    Absent,
    Configured {
        remote: String,
        reference: RefName,
        commit: RevisionId,
    },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitFacts {
    head: HeadState,
    upstream: UpstreamState,
    index: IndexState,
    worktree: WorktreeState,
}

impl GitFacts {
    pub fn new(
        head: HeadState,
        upstream: UpstreamState,
        index: IndexState,
        worktree: WorktreeState,
    ) -> Result<Self, DomainError> {
        if let UpstreamState::Configured { remote, .. } = &upstream {
            validate_text(remote, "upstream remote")?;
        }
        Ok(Self {
            head,
            upstream,
            index: index.normalize()?,
            worktree: worktree.normalize()?,
        })
    }

    pub fn head(&self) -> &HeadState {
        &self.head
    }

    pub fn upstream(&self) -> &UpstreamState {
        &self.upstream
    }

    pub fn index(&self) -> &IndexState {
        &self.index
    }

    pub fn worktree(&self) -> &WorktreeState {
        &self.worktree
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GitRepositoryState {
    NonGit,
    Git(GitFacts),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositoryFacts {
    repository_id: RepositoryId,
    root: RepositoryRoot,
    git: GitRepositoryState,
}

impl RepositoryFacts {
    pub fn new(
        repository_id: RepositoryId,
        root: RepositoryRoot,
        git: GitRepositoryState,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            repository_id,
            root,
            git,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn root(&self) -> &RepositoryRoot {
        &self.root
    }

    pub fn git(&self) -> &GitRepositoryState {
        &self.git
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrozenWitnesses {
    authority: String,
    source: String,
    catalog: String,
    configuration: String,
    plan: String,
    checks: Vec<CheckWitness>,
    base_head: Option<RevisionId>,
}

impl FrozenWitnesses {
    pub fn new(
        authority: impl Into<String>,
        source: impl Into<String>,
        catalog: impl Into<String>,
        configuration: impl Into<String>,
        plan: impl Into<String>,
        checks: Vec<CheckWitness>,
        base_head: Option<RevisionId>,
    ) -> Result<Self, DomainError> {
        let authority = authority.into();
        let source = source.into();
        let catalog = catalog.into();
        let configuration = configuration.into();
        let plan = plan.into();
        validate_text(&authority, "authority witness")?;
        validate_text(&source, "source witness")?;
        validate_text(&catalog, "catalog witness")?;
        validate_text(&configuration, "configuration witness")?;
        validate_text(&plan, "plan witness")?;
        for (index, check) in checks.iter().enumerate() {
            if checks[..index].contains(check) {
                return Err(DomainError::DuplicateValue {
                    field: "check witness",
                    value: check.as_str().to_owned(),
                });
            }
        }
        Ok(Self {
            authority,
            source,
            catalog,
            configuration,
            plan,
            checks,
            base_head,
        })
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn catalog(&self) -> &str {
        &self.catalog
    }

    pub fn configuration(&self) -> &str {
        &self.configuration
    }

    pub fn plan(&self) -> &str {
        &self.plan
    }

    pub fn checks(&self) -> &[CheckWitness] {
        &self.checks
    }

    pub fn base_head(&self) -> Option<&RevisionId> {
        self.base_head.as_ref()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositorySnapshot {
    facts: RepositoryFacts,
    witnesses: FrozenWitnesses,
    targets: Vec<ManagedTargetIdentity>,
}

impl RepositorySnapshot {
    pub fn new(
        facts: RepositoryFacts,
        witnesses: FrozenWitnesses,
        mut targets: Vec<ManagedTargetIdentity>,
    ) -> Result<Self, DomainError> {
        targets.sort();
        for pair in targets.windows(2) {
            if pair[0] == pair[1] {
                return Err(DomainError::DuplicateValue {
                    field: "managed target",
                    value: String::from_utf8_lossy(pair[0].path().as_bytes()).into_owned(),
                });
            }
            if pair[0].conflicts_with(&pair[1]) {
                return Err(DomainError::ConflictingTarget {
                    path: String::from_utf8_lossy(pair[0].path().as_bytes()).into_owned(),
                });
            }
        }
        Ok(Self {
            facts,
            witnesses,
            targets,
        })
    }

    pub fn facts(&self) -> &RepositoryFacts {
        &self.facts
    }

    pub fn identity(&self) -> AuthorityIdentity {
        self.facts.root().authority().clone()
    }

    pub fn witnesses(&self) -> &FrozenWitnesses {
        &self.witnesses
    }

    pub fn targets(&self) -> &[ManagedTargetIdentity] {
        &self.targets
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthorizedChange {
    target: ManagedTargetIdentity,
    change: TargetChange,
    before: Option<FileIdentity>,
    after: Option<FileIdentity>,
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
    repository_id: RepositoryId,
    authority_identity: AuthorityIdentity,
    snapshot_identity: CanonicalRepresentation,
    witnesses: FrozenWitnesses,
    frozen_targets: Vec<ManagedTargetIdentity>,
    base_head: Option<RevisionId>,
    changes: Vec<AuthorizedChange>,
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObservedFact<T>(T);

impl<T> ObservedFact<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &T {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OwnerDecision<T>(T);

impl<T> OwnerDecision<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &T {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausationRelation {
    Direct,
    Unrelated,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausationBasis {
    BaselineComparison,
    FailureEvidence,
    NotEstablished,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BaselineIdentityProof {
    before: AuthorityIdentity,
    after: AuthorityIdentity,
    before_snapshot: CanonicalRepresentation,
    after_snapshot: CanonicalRepresentation,
}

impl BaselineIdentityProof {
    pub fn from_snapshot(
        expected: &RepositorySnapshot,
        observed: &RepositorySnapshot,
    ) -> Result<Self, DomainError> {
        let before_snapshot = expected.canonical_representation();
        let after_snapshot = observed.canonical_representation();
        if before_snapshot != after_snapshot {
            return Err(DomainError::InvalidCausation {
                relation: CausationRelation::Direct,
                basis: CausationBasis::BaselineComparison,
            });
        }
        Ok(Self {
            before: expected.identity(),
            after: observed.identity(),
            before_snapshot,
            after_snapshot,
        })
    }

    pub fn before(&self) -> &AuthorityIdentity {
        &self.before
    }

    pub fn after(&self) -> &AuthorityIdentity {
        &self.after
    }

    pub fn before_snapshot(&self) -> &CanonicalRepresentation {
        &self.before_snapshot
    }

    pub fn after_snapshot(&self) -> &CanonicalRepresentation {
        &self.after_snapshot
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedPathFailureProof {
    snapshot_identity: CanonicalRepresentation,
    operation: CanonicalRepresentation,
    target: ManagedTargetIdentity,
    failure: String,
}

impl ManagedPathFailureProof {
    pub fn new(
        snapshot: &RepositorySnapshot,
        operation: &AuthorizedDelta,
        target: ManagedTargetIdentity,
        failure: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let failure = failure.into();
        validate_text(&failure, "managed path failure")?;
        let snapshot_identity = snapshot.canonical_representation();
        if operation.snapshot_identity() != &snapshot_identity {
            return Err(DomainError::InvalidProofBinding { field: "snapshot" });
        }
        if !operation.frozen_targets().contains(&target)
            || !operation
                .changes()
                .iter()
                .any(|change| change.target() == &target)
        {
            return Err(DomainError::InvalidProofBinding { field: "operation" });
        }
        Ok(Self {
            snapshot_identity,
            operation: operation.canonical_representation(),
            target,
            failure,
        })
    }

    pub fn snapshot_identity(&self) -> &CanonicalRepresentation {
        &self.snapshot_identity
    }

    pub fn operation(&self) -> &CanonicalRepresentation {
        &self.operation
    }

    pub fn target(&self) -> &ManagedTargetIdentity {
        &self.target
    }

    pub fn failure(&self) -> &str {
        &self.failure
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DirectCausationProof {
    Baseline(BaselineIdentityProof),
    ManagedPath(ManagedPathFailureProof),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InferredCausation {
    relation: CausationRelation,
    proof: Option<DirectCausationProof>,
}

pub type CausationAssessment = InferredCausation;

impl InferredCausation {
    pub fn new(relation: CausationRelation, basis: CausationBasis) -> Result<Self, DomainError> {
        if relation == CausationRelation::Direct || !matches!(basis, CausationBasis::NotEstablished)
        {
            return Err(DomainError::InvalidCausation { relation, basis });
        }
        Ok(Self {
            relation,
            proof: None,
        })
    }

    pub fn direct(proof: DirectCausationProof) -> Self {
        Self {
            relation: CausationRelation::Direct,
            proof: Some(proof),
        }
    }

    pub fn try_direct_without_proof() -> Result<Self, DomainError> {
        Err(DomainError::InvalidCausation {
            relation: CausationRelation::Direct,
            basis: CausationBasis::NotEstablished,
        })
    }

    pub fn uncertain() -> Self {
        Self {
            relation: CausationRelation::Uncertain,
            proof: None,
        }
    }

    pub fn relation(&self) -> CausationRelation {
        self.relation
    }

    pub fn basis(&self) -> CausationBasis {
        match self.proof.as_ref() {
            Some(DirectCausationProof::Baseline(_)) => CausationBasis::BaselineComparison,
            Some(DirectCausationProof::ManagedPath(_)) => CausationBasis::FailureEvidence,
            None => CausationBasis::NotEstablished,
        }
    }

    pub fn proof(&self) -> Option<&DirectCausationProof> {
        self.proof.as_ref()
    }

    pub fn is_repair_eligible(&self) -> bool {
        matches!(self.relation, CausationRelation::Direct) && self.proof.is_some()
    }
}

/// Version for the explicit, tagged, length-delimited repository-state wire
/// representation.  A format change increments this value; no decoder or
/// compatibility layer is kept in this pure domain module.
pub const CANONICAL_REPOSITORY_STATE_VERSION: u16 = 2;

const CANONICAL_MAGIC: &[u8] = b"OMNI";
const DOCUMENT_SNAPSHOT: u8 = 1;
const DOCUMENT_DELTA: u8 = 2;

const RECORD_FACTS: u8 = 0x10;
const RECORD_ROOT: u8 = 0x11;
const RECORD_AUTHORITY: u8 = 0x12;
const RECORD_FILESYSTEM: u8 = 0x13;
const RECORD_FILESYSTEM_CLASS: u8 = 0x14;
const RECORD_OBJECT: u8 = 0x15;
const RECORD_FILE: u8 = 0x16;
const RECORD_TARGET: u8 = 0x17;
const RECORD_OWNERSHIP: u8 = 0x18;
const RECORD_INDEX_ENTRY: u8 = 0x19;
const RECORD_WORKTREE_ENTRY: u8 = 0x1a;
const RECORD_INDEX_STATE: u8 = 0x1b;
const RECORD_WORKTREE_STATE: u8 = 0x1c;
const RECORD_HEAD: u8 = 0x1d;
const RECORD_UPSTREAM: u8 = 0x1e;
const RECORD_GIT_FACTS: u8 = 0x1f;
const RECORD_GIT_STATE: u8 = 0x20;
const RECORD_WITNESSES: u8 = 0x21;
const RECORD_TARGETS: u8 = 0x22;
const RECORD_CHANGES: u8 = 0x23;
const RECORD_CHANGE: u8 = 0x24;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalRepresentation {
    version: u16,
    bytes: Vec<u8>,
}

impl CanonicalRepresentation {
    fn new(document: u8, fields: Vec<(u8, Vec<u8>)>) -> Self {
        let mut bytes = Vec::with_capacity(CANONICAL_MAGIC.len() + 3);
        bytes.extend_from_slice(CANONICAL_MAGIC);
        bytes.extend_from_slice(&CANONICAL_REPOSITORY_STATE_VERSION.to_be_bytes());
        bytes.push(document);
        append_fields(&mut bytes, &fields);
        Self {
            version: CANONICAL_REPOSITORY_STATE_VERSION,
            bytes,
        }
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp(other)
    }
}

fn append_field(bytes: &mut Vec<u8>, tag: u8, value: &[u8]) {
    bytes.push(tag);
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}

fn append_fields(bytes: &mut Vec<u8>, fields: &[(u8, Vec<u8>)]) {
    for (tag, value) in fields {
        append_field(bytes, *tag, value);
    }
}

fn record(tag: u8, fields: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = vec![tag];
    append_fields(&mut bytes, &fields);
    bytes
}

fn sequence(tag: u8, values: Vec<Vec<u8>>) -> Vec<u8> {
    record(tag, values.into_iter().map(|value| (1, value)).collect())
}

fn optional(value: Option<Vec<u8>>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(value) => {
            let mut bytes = Vec::with_capacity(value.len() + 1);
            bytes.push(1);
            bytes.extend_from_slice(&value);
            bytes
        }
    }
}

fn u8_bytes(value: u8) -> Vec<u8> {
    vec![value]
}

fn u32_bytes(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn u64_bytes(value: u64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn text_bytes(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

fn encode_filesystem_class(class: &FilesystemClass) -> Vec<u8> {
    match class {
        FilesystemClass::LinuxExtFamily => record(RECORD_FILESYSTEM_CLASS, vec![(1, vec![0])]),
        FilesystemClass::MacOsApfs => record(RECORD_FILESYSTEM_CLASS, vec![(1, vec![1])]),
        FilesystemClass::Other(name) => record(
            RECORD_FILESYSTEM_CLASS,
            vec![(1, vec![2]), (2, text_bytes(name.as_str()))],
        ),
    }
}

fn encode_filesystem(identity: &FilesystemIdentity) -> Vec<u8> {
    record(
        RECORD_FILESYSTEM,
        vec![
            (1, encode_filesystem_class(identity.class())),
            (2, u64_bytes(identity.device())),
            (3, u64_bytes(identity.mount_id())),
        ],
    )
}

fn encode_object(identity: ObjectIdentity) -> Vec<u8> {
    record(
        RECORD_OBJECT,
        vec![
            (1, u64_bytes(identity.device())),
            (2, u64_bytes(identity.inode())),
        ],
    )
}

fn encode_authority(identity: &AuthorityIdentity) -> Vec<u8> {
    record(
        RECORD_AUTHORITY,
        vec![
            (1, encode_filesystem(identity.filesystem())),
            (2, encode_object(identity.object())),
        ],
    )
}

fn encode_root(root: &RepositoryRoot) -> Vec<u8> {
    record(
        RECORD_ROOT,
        vec![
            (1, text_bytes(root.as_str())),
            (2, encode_authority(root.authority())),
        ],
    )
}

fn encode_file(identity: &FileIdentity) -> Vec<u8> {
    let kind = match identity.kind() {
        EntryKind::RegularFile => 0,
        EntryKind::Directory => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    };
    record(
        RECORD_FILE,
        vec![
            (1, encode_filesystem(identity.filesystem())),
            (2, encode_object(*identity.object())),
            (3, u8_bytes(kind)),
            (4, u32_bytes(identity.mode())),
        ],
    )
}

fn encode_ownership(ownership: &ManagedOwnership) -> Vec<u8> {
    match ownership {
        ManagedOwnership::WholeFile => record(RECORD_OWNERSHIP, vec![(1, vec![0])]),
        ManagedOwnership::Section { id } => record(
            RECORD_OWNERSHIP,
            vec![(1, vec![1]), (2, text_bytes(id.as_str()))],
        ),
    }
}

fn encode_target(target: &ManagedTargetIdentity) -> Vec<u8> {
    record(
        RECORD_TARGET,
        vec![
            (1, target.path().as_bytes().to_vec()),
            (2, encode_ownership(target.ownership())),
            (3, optional(target.observed_file().map(encode_file))),
        ],
    )
}

fn encode_target_change(change: TargetChange) -> Vec<u8> {
    let value = match change {
        TargetChange::Added => 0,
        TargetChange::Deleted => 1,
        TargetChange::Modified => 2,
        TargetChange::Renamed => 3,
        TargetChange::TypeChanged => 4,
        TargetChange::ModeChanged => 5,
        TargetChange::LinkChanged => 6,
        TargetChange::Untracked => 7,
    };
    u8_bytes(value)
}

fn encode_provenance(provenance: DirtyProvenance) -> Vec<u8> {
    u8_bytes(match provenance {
        DirtyProvenance::PreExisting => 0,
        DirtyProvenance::CurrentOperation => 1,
    })
}

fn encode_index_entry(entry: &IndexEntry) -> Vec<u8> {
    record(
        RECORD_INDEX_ENTRY,
        vec![
            (1, entry.path().as_bytes().to_vec()),
            (2, encode_target_change(entry.change())),
            (3, encode_provenance(entry.provenance())),
            (
                4,
                optional(entry.rename_from().map(|path| path.as_bytes().to_vec())),
            ),
        ],
    )
}

fn encode_worktree_entry(entry: &WorktreeEntry) -> Vec<u8> {
    record(
        RECORD_WORKTREE_ENTRY,
        vec![
            (1, entry.path().as_bytes().to_vec()),
            (2, encode_target_change(entry.change())),
            (3, encode_provenance(entry.provenance())),
            (
                4,
                optional(entry.rename_from().map(|path| path.as_bytes().to_vec())),
            ),
        ],
    )
}

fn encode_index_state(state: &IndexState) -> Vec<u8> {
    match state {
        IndexState::Clean => record(RECORD_INDEX_STATE, vec![(1, vec![0])]),
        IndexState::Entries(entries) => record(
            RECORD_INDEX_STATE,
            vec![
                (1, vec![1]),
                (
                    2,
                    sequence(
                        RECORD_CHANGES,
                        entries.iter().map(encode_index_entry).collect(),
                    ),
                ),
            ],
        ),
    }
}

fn encode_worktree_state(state: &WorktreeState) -> Vec<u8> {
    match state {
        WorktreeState::Clean => record(RECORD_WORKTREE_STATE, vec![(1, vec![0])]),
        WorktreeState::Entries(entries) => record(
            RECORD_WORKTREE_STATE,
            vec![
                (1, vec![1]),
                (
                    2,
                    sequence(
                        RECORD_CHANGES,
                        entries.iter().map(encode_worktree_entry).collect(),
                    ),
                ),
            ],
        ),
    }
}

fn encode_head(head: &HeadState) -> Vec<u8> {
    match head {
        HeadState::Unborn => record(RECORD_HEAD, vec![(1, vec![0])]),
        HeadState::Detached { commit } => record(
            RECORD_HEAD,
            vec![(1, vec![1]), (3, text_bytes(commit.as_str()))],
        ),
        HeadState::Attached { branch, commit } => record(
            RECORD_HEAD,
            vec![
                (1, vec![2]),
                (2, text_bytes(branch.as_str())),
                (3, text_bytes(commit.as_str())),
            ],
        ),
    }
}

fn encode_upstream(upstream: &UpstreamState) -> Vec<u8> {
    match upstream {
        UpstreamState::Absent => record(RECORD_UPSTREAM, vec![(1, vec![0])]),
        UpstreamState::Configured {
            remote,
            reference,
            commit,
        } => record(
            RECORD_UPSTREAM,
            vec![
                (1, vec![1]),
                (2, text_bytes(remote)),
                (3, text_bytes(reference.as_str())),
                (4, text_bytes(commit.as_str())),
            ],
        ),
    }
}

fn encode_git_facts(facts: &GitFacts) -> Vec<u8> {
    record(
        RECORD_GIT_FACTS,
        vec![
            (1, encode_head(facts.head())),
            (2, encode_upstream(facts.upstream())),
            (3, encode_index_state(facts.index())),
            (4, encode_worktree_state(facts.worktree())),
        ],
    )
}

fn encode_git_state(state: &GitRepositoryState) -> Vec<u8> {
    match state {
        GitRepositoryState::NonGit => record(RECORD_GIT_STATE, vec![(1, vec![0])]),
        GitRepositoryState::Git(facts) => record(
            RECORD_GIT_STATE,
            vec![(1, vec![1]), (2, encode_git_facts(facts))],
        ),
    }
}

fn encode_repository_facts(facts: &RepositoryFacts) -> Vec<u8> {
    record(
        RECORD_FACTS,
        vec![
            (1, text_bytes(facts.repository_id().as_str())),
            (2, encode_root(facts.root())),
            (3, encode_git_state(facts.git())),
        ],
    )
}

fn encode_witnesses(witnesses: &FrozenWitnesses) -> Vec<u8> {
    record(
        RECORD_WITNESSES,
        vec![
            (1, text_bytes(witnesses.authority())),
            (2, text_bytes(witnesses.source())),
            (3, text_bytes(witnesses.catalog())),
            (4, text_bytes(witnesses.configuration())),
            (5, text_bytes(witnesses.plan())),
            (
                6,
                sequence(
                    RECORD_CHANGES,
                    witnesses
                        .checks()
                        .iter()
                        .map(|check| text_bytes(check.as_str()))
                        .collect(),
                ),
            ),
            (
                7,
                optional(
                    witnesses
                        .base_head()
                        .map(|revision| text_bytes(revision.as_str())),
                ),
            ),
        ],
    )
}

fn encode_authorized_change(change: &AuthorizedChange) -> Vec<u8> {
    record(
        RECORD_CHANGE,
        vec![
            (1, encode_target(change.target())),
            (2, encode_target_change(change.change())),
            (3, optional(change.before().map(encode_file))),
            (4, optional(change.after().map(encode_file))),
            (
                5,
                optional(change.rename_from().map(|path| path.as_bytes().to_vec())),
            ),
        ],
    )
}

impl RepositorySnapshot {
    pub fn canonical_representation(&self) -> CanonicalRepresentation {
        CanonicalRepresentation::new(
            DOCUMENT_SNAPSHOT,
            vec![
                (1, encode_repository_facts(&self.facts)),
                (2, encode_witnesses(&self.witnesses)),
                (
                    3,
                    sequence(
                        RECORD_TARGETS,
                        self.targets.iter().map(encode_target).collect(),
                    ),
                ),
            ],
        )
    }
}

impl AuthorizedDelta {
    pub fn canonical_representation(&self) -> CanonicalRepresentation {
        CanonicalRepresentation::new(
            DOCUMENT_DELTA,
            vec![
                (1, text_bytes(self.repository_id.as_str())),
                (2, encode_authority(&self.authority_identity)),
                (3, self.snapshot_identity.as_bytes().to_vec()),
                (4, encode_witnesses(&self.witnesses)),
                (
                    5,
                    sequence(
                        RECORD_TARGETS,
                        self.frozen_targets.iter().map(encode_target).collect(),
                    ),
                ),
                (
                    6,
                    optional(
                        self.base_head
                            .as_ref()
                            .map(|revision| text_bytes(revision.as_str())),
                    ),
                ),
                (
                    7,
                    sequence(
                        RECORD_CHANGES,
                        self.changes.iter().map(encode_authorized_change).collect(),
                    ),
                ),
            ],
        )
    }
}
