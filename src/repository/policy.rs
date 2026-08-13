//! Immutable destination-repository policy domain types.
//!
//! This module deliberately stops at the policy boundary.  It does not find
//! files, parse YAML, infer source content, execute commands, or mutate a
//! repository.  Adapters provide a validated policy and an opaque authority
//! identity; this module keeps the resulting state explicit and immutable.

use std::{collections::HashSet, error::Error, fmt};

/// The only schema version accepted by the first constitutional release.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SchemaVersion(u64);

impl SchemaVersion {
    pub const V1: Self = Self(1);

    pub const fn current() -> Self {
        Self::V1
    }

    pub fn new(value: u64) -> Result<Self, PolicyError> {
        if value == Self::V1.value() {
            Ok(Self(value))
        } else {
            Err(PolicyError::UnsupportedSchemaVersion {
                found: value,
                supported: Self::V1,
            })
        }
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// A stable, exact managed-item identifier.
///
/// The parser/loader owns the authority file.  This type only accepts the
/// already-decided lowercase ASCII slug grammar so an invalid ID cannot enter
/// a valid policy snapshot.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedItemId(String);

impl ManagedItemId {
    pub fn new(value: impl Into<String>) -> Result<Self, PolicyError> {
        let value = value.into();
        if value.is_empty()
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
        {
            return Err(PolicyError::InvalidManagedItemId { value });
        }

        Ok(Self(value))
    }

    pub fn parse(value: &str) -> Result<Self, PolicyError> {
        Self::new(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ManagedItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Selection intent declared by a destination repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionPolicy {
    state: SelectionState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SelectionState {
    /// The selector keys were omitted.  This remains present policy and
    /// therefore selects nothing; it must never trigger inference.
    Omitted,
    /// Explicit selection.  The effective set is `(all applicable OR allow)`
    /// minus `exclude`; exclusion wins when an ID appears in both lists.
    Explicit {
        all: bool,
        allow: Vec<ManagedItemId>,
        exclude: Vec<ManagedItemId>,
    },
}

impl SelectionPolicy {
    pub const fn omitted() -> Self {
        Self {
            state: SelectionState::Omitted,
        }
    }

    pub fn explicit<A, E>(all: bool, allow: A, exclude: E) -> Result<Self, PolicyError>
    where
        A: IntoIterator<Item = ManagedItemId>,
        E: IntoIterator<Item = ManagedItemId>,
    {
        let allow = collect_unique("allow", allow)?;
        let exclude = collect_unique("exclude", exclude)?;
        Ok(Self {
            state: SelectionState::Explicit {
                all,
                allow,
                exclude,
            },
        })
    }

    pub const fn is_omitted(&self) -> bool {
        matches!(self.state, SelectionState::Omitted)
    }

    pub const fn all(&self) -> bool {
        match &self.state {
            SelectionState::Omitted => false,
            SelectionState::Explicit { all, .. } => *all,
        }
    }

    pub fn allow(&self) -> &[ManagedItemId] {
        match &self.state {
            SelectionState::Omitted => &[],
            SelectionState::Explicit { allow, .. } => allow,
        }
    }

    pub fn exclude(&self) -> &[ManagedItemId] {
        match &self.state {
            SelectionState::Omitted => &[],
            SelectionState::Explicit { exclude, .. } => exclude,
        }
    }

    /// Whether this declaration selects no managed content by itself.
    pub fn selects_nothing(&self) -> bool {
        !self.all() && self.allow().is_empty()
    }
}

fn collect_unique<I>(field: &'static str, values: I) -> Result<Vec<ManagedItemId>, PolicyError>
where
    I: IntoIterator<Item = ManagedItemId>,
{
    let mut seen = HashSet::new();
    let mut collected = Vec::new();
    for value in values {
        if !seen.insert(value.clone()) {
            return Err(PolicyError::DuplicateSelector { field, id: value });
        }
        collected.push(value);
    }
    Ok(collected)
}

/// One shell-free verification command's immutable argv.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationCommand {
    argv: Vec<String>,
}

impl VerificationCommand {
    pub fn new<I, S>(args: I) -> Result<Self, PolicyError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let argv = args.into_iter().map(Into::into).collect::<Vec<_>>();
        if argv.first().is_none_or(String::is_empty) {
            return Err(PolicyError::EmptyCommandExecutable);
        }
        if let Some(index) = argv
            .iter()
            .position(|argument| argument.as_bytes().contains(&0))
        {
            return Err(PolicyError::NulInCommandArgument { index });
        }
        Ok(Self { argv })
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }
}

/// Presence of the repository's command field is retained independently from
/// its contents.  `Present(vec![])` is intentional empty configuration, not
/// an absent repository policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicy {
    state: CommandState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommandState {
    Absent,
    Present(Vec<VerificationCommand>),
}

impl CommandPolicy {
    pub const fn absent() -> Self {
        Self {
            state: CommandState::Absent,
        }
    }

    pub fn present(commands: Vec<VerificationCommand>) -> Result<Self, PolicyError> {
        for (duplicate, command) in commands.iter().enumerate() {
            if let Some(first) = commands[..duplicate]
                .iter()
                .position(|previous| previous == command)
            {
                return Err(PolicyError::DuplicateCommand { first, duplicate });
            }
        }
        Ok(Self {
            state: CommandState::Present(commands),
        })
    }

    pub const fn is_absent(&self) -> bool {
        matches!(self.state, CommandState::Absent)
    }

    pub const fn is_present(&self) -> bool {
        matches!(self.state, CommandState::Present(_))
    }

    pub fn as_slice(&self) -> Option<&[VerificationCommand]> {
        match &self.state {
            CommandState::Absent => None,
            CommandState::Present(commands) => Some(commands),
        }
    }
}

/// The validated, destination-owned policy.  It has no fields for fleet
/// membership, source authority, or source priority; those concepts belong to
/// separate authority domains and cannot be smuggled into this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPolicy {
    schema_version: SchemaVersion,
    selection: SelectionPolicy,
    commands: CommandPolicy,
}

impl RepositoryPolicy {
    pub fn new(
        schema_version: SchemaVersion,
        selection: SelectionPolicy,
        commands: CommandPolicy,
    ) -> Result<Self, PolicyError> {
        if schema_version != SchemaVersion::V1 {
            return Err(PolicyError::UnsupportedSchemaVersion {
                found: schema_version.value(),
                supported: SchemaVersion::V1,
            });
        }
        Ok(Self {
            schema_version,
            selection,
            commands,
        })
    }

    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    pub const fn selection(&self) -> &SelectionPolicy {
        &self.selection
    }

    pub const fn commands(&self) -> &CommandPolicy {
        &self.commands
    }
}

/// Opaque authority identity captured when a policy file is read.
///
/// Filesystem adapters must derive this from the complete no-follow authority
/// identity and can use any stable byte representation.  The domain never
/// hashes, reads, compares, or mutates the file itself.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PolicyIdentity {
    bytes: [u8; 32],
}

impl PolicyIdentity {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.bytes
    }
}

/// A policy plus the identity against which later reads must be revalidated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPolicySnapshot {
    identity: PolicyIdentity,
    policy: RepositoryPolicy,
}

impl RepositoryPolicySnapshot {
    pub const fn identity(&self) -> PolicyIdentity {
        self.identity
    }

    pub const fn policy(&self) -> &RepositoryPolicy {
        &self.policy
    }

    pub fn revalidate(&self, observed: PolicyIdentity) -> Result<(), PolicySnapshotError> {
        if self.identity == observed {
            Ok(())
        } else {
            Err(PolicySnapshotError::Changed {
                expected: self.identity,
                observed,
            })
        }
    }
}

/// Why a valid snapshot can no longer be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicySnapshotError {
    Changed {
        expected: PolicyIdentity,
        observed: PolicyIdentity,
    },
}

/// The complete result of policy discovery and validation.
///
/// Only [`RepositoryPolicyState::Absent`] enables later inference.  Every
/// other state is intentional presence or an explicit failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositoryPolicyState {
    Absent,
    Present(RepositoryPolicySnapshot),
    Invalid(PolicyError),
    Changed(PolicySnapshotError),
}

impl RepositoryPolicyState {
    pub const fn absent() -> Self {
        Self::Absent
    }

    pub fn present(identity: PolicyIdentity, policy: RepositoryPolicy) -> Self {
        Self::Present(RepositoryPolicySnapshot { identity, policy })
    }

    pub const fn invalid(error: PolicyError) -> Self {
        Self::Invalid(error)
    }

    pub const fn changed(error: PolicySnapshotError) -> Self {
        Self::Changed(error)
    }

    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub const fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }

    pub const fn as_snapshot(&self) -> Option<&RepositoryPolicySnapshot> {
        match self {
            Self::Present(snapshot) => Some(snapshot),
            Self::Absent | Self::Invalid(_) | Self::Changed(_) => None,
        }
    }

    pub const fn error(&self) -> Option<&PolicyError> {
        match self {
            Self::Invalid(error) => Some(error),
            Self::Absent | Self::Present(_) | Self::Changed(_) => None,
        }
    }

    pub const fn snapshot_error(&self) -> Option<&PolicySnapshotError> {
        match self {
            Self::Changed(error) => Some(error),
            Self::Absent | Self::Present(_) | Self::Invalid(_) => None,
        }
    }
}

/// Validation failures that are not snapshot replacement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    InvalidManagedItemId {
        value: String,
    },
    DuplicateSelector {
        field: &'static str,
        id: ManagedItemId,
    },
    EmptyCommandExecutable,
    NulInCommandArgument {
        index: usize,
    },
    DuplicateCommand {
        first: usize,
        duplicate: usize,
    },
    UnsupportedSchemaVersion {
        found: u64,
        supported: SchemaVersion,
    },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManagedItemId { value } => {
                write!(formatter, "invalid managed-item ID {value:?}")
            }
            Self::DuplicateSelector { field, id } => {
                write!(formatter, "duplicate {field} selector {id:?}")
            }
            Self::EmptyCommandExecutable => {
                formatter.write_str("verification command executable cannot be empty")
            }
            Self::NulInCommandArgument { index } => {
                write!(
                    formatter,
                    "verification command argument {index} contains NUL"
                )
            }
            Self::DuplicateCommand { first, duplicate } => write!(
                formatter,
                "verification command {duplicate} duplicates command {first} exactly"
            ),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "unsupported repository policy schema version {found}; expected {}",
                supported.value()
            ),
        }
    }
}

impl Error for PolicyError {}

impl fmt::Display for PolicySnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed { expected, observed } => write!(
                formatter,
                "repository policy changed during snapshot (expected {:?}, observed {:?})",
                expected.as_bytes(),
                observed.as_bytes()
            ),
        }
    }
}

impl Error for PolicySnapshotError {}
