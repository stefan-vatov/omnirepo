//! Typed machine-level configuration authority.
//!
//! This module contains data and validation only.  It does not discover files,
//! read YAML, inspect a filesystem, run Git, or execute repository commands.
//! Those effects belong to the configuration and lifecycle consumers.  Keeping
//! the authority model separate makes it impossible for a repository policy or
//! source declaration to become a machine-level field by accident.

#![allow(dead_code)]

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{error::Error, fmt};

#[cfg(test)]
mod unit_tests;

mod discovery;
mod section_id;
mod yaml_subset;

pub(crate) use discovery::{Discovery, discover};
pub(crate) use section_id::{SectionId, is_valid_section_id};
pub(crate) use yaml_subset::{YValue, parse_yaml_subset};

#[cfg(test)]
mod discovery_tests;

#[cfg(test)]
mod yaml_subset_tests;

/// Constitutional synchronization surface. The owner-approved command tree
/// is exactly `sync`, `setup`, and `doctor`; there is no `migrate`
/// command in the first constitutional release, and legacy general surfaces
/// (run/new/clone/ad-hoc sync) are absent by boundary decision.
#[derive(Debug, Parser)]
#[command(
    name = "omnirepo",
    version,
    about = "Synchronize machine-declared managed content into destination repositories",
    long_about = None,
    after_help = "Commands: sync, setup, doctor.\nMachine configuration: <HOME>/.omnirepo/config.yaml (YAML version: 1).\nLegacy general orchestration surfaces are unsupported and are not migrated automatically."
)]
struct Cli {
    /// Emit a versioned machine-readable JSON projection of the outcome.
    #[arg(long, global = true, value_enum, default_value_t = OutputMode::Human)]
    output: OutputMode,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputMode {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Synchronize managed files and sections from machine-declared sources
    /// into selected destination repositories.
    Sync,
    /// Set up machine configuration for a first synchronization.
    Setup(SetupArgs),
    /// Diagnose the machine: configuration, sources, declarations, and
    /// cross-source conflicts, without any destination effect.
    Doctor,
}

#[derive(Debug, Args)]
pub(crate) struct SetupArgs {
    /// Apply the setup plan instead of printing it.
    #[arg(long)]
    apply: bool,
}

pub(crate) fn parse() -> Command {
    Cli::parse().command
}

/// The canonical public command surface of the binary, in declared order.
pub(crate) fn command_surface() -> Vec<String> {
    use clap::CommandFactory;
    Cli::command()
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_owned())
        .collect()
}

pub const SUPPORTED_SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_MAX_REPOSITORIES: u16 = 4;
pub const DEFAULT_MAX_CHILD_WORK: u16 = 8;
pub const MAX_REPOSITORIES: u16 = 32;
pub const MAX_CHILD_WORK: u16 = 64;
pub const DEFAULT_REPAIR_ATTEMPTS: u8 = 3;
pub const MAX_REPAIR_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SchemaVersion(u16);

impl SchemaVersion {
    pub const fn current() -> Self {
        Self(SUPPORTED_SCHEMA_VERSION)
    }

    pub fn new(value: u16) -> Result<Self, ConfigurationError> {
        if value == SUPPORTED_SCHEMA_VERSION {
            Ok(Self(value))
        } else {
            Err(ConfigurationError::UnsupportedSchemaVersion {
                expected: SUPPORTED_SCHEMA_VERSION,
                actual: value,
            })
        }
    }

    pub const fn value(self) -> u16 {
        self.0
    }
}

macro_rules! slug_type {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> Result<Self, ConfigurationError> {
                validate_slug($field, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

slug_type!(SourceId, "source id");
slug_type!(RepositoryTag, "repository tag");

/// A destination repository identity is an opaque printable label.  It is
/// not a managed-item slug: repository names may contain spaces or uppercase
/// text.  The identity is still lexical; filesystem identity is checked by
/// the authority adapter.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositoryId(String);

impl RepositoryId {
    pub fn parse(value: &str) -> Result<Self, ConfigurationError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(ConfigurationError::InvalidIdentityText {
                field: "repository id",
                value: value.to_owned(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for RepositoryId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

fn validate_slug(field: &'static str, value: &str) -> Result<String, ConfigurationError> {
    let valid = !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        });
    if valid {
        Ok(value.to_owned())
    } else {
        Err(ConfigurationError::InvalidSlug {
            field,
            value: value.to_owned(),
        })
    }
}

/// An absolute UTF-8 path in the configured portable `/` representation.
///
/// This is a lexical value.  Construction rejects relative and parent-
/// traversing text, but it does not resolve `.` components, symlinks, mount
/// crossings, or filesystem aliases.  The authority adapter owns that
/// platform-specific identity check before any effect.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AbsolutePath(String);

impl AbsolutePath {
    pub fn parse(value: &str) -> Result<Self, ConfigurationError> {
        if value.is_empty() {
            return Err(ConfigurationError::InvalidPath {
                field: "path",
                reason: "path is empty",
            });
        }
        if !value.starts_with('/') {
            return Err(ConfigurationError::InvalidPath {
                field: "path",
                reason: "path must be absolute",
            });
        }
        if value.as_bytes().contains(&0) {
            return Err(ConfigurationError::InvalidPath {
                field: "path",
                reason: "path contains NUL",
            });
        }
        if value.split('/').any(|component| component == "..") {
            return Err(ConfigurationError::InvalidPath {
                field: "path",
                reason: "parent traversal is not allowed",
            });
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for AbsolutePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A local or remote Git source locator.
///
/// Local paths retain the same lexical-only scope as [`AbsolutePath`].  A
/// source adapter resolves the actual filesystem and Git identity later.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceLocation {
    Local(AbsolutePath),
    Remote(String),
}

impl SourceLocation {
    pub fn local(path: AbsolutePath) -> Self {
        Self::Local(path)
    }

    pub fn remote(value: &str) -> Result<Self, ConfigurationError> {
        let supported = value.starts_with("https://")
            || value.starts_with("ssh://")
            || (value.starts_with("git@") && value.contains(':'));
        if value.is_empty() || value.as_bytes().contains(&0) || !supported {
            return Err(ConfigurationError::InvalidSourceLocation {
                reason: "source must use HTTPS or SSH",
            });
        }
        Ok(Self::Remote(value.to_owned()))
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local(_))
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Local(path) => path.as_str(),
            Self::Remote(url) => url,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestinationRepository {
    id: RepositoryId,
    path: AbsolutePath,
    tags: Vec<RepositoryTag>,
}

impl DestinationRepository {
    pub fn new<I>(id: RepositoryId, path: AbsolutePath, tags: I) -> Result<Self, ConfigurationError>
    where
        I: IntoIterator<Item = RepositoryTag>,
    {
        let tags = tags.into_iter().collect::<Vec<_>>();
        for (index, tag) in tags.iter().enumerate() {
            if tags[..index].iter().any(|previous| previous == tag) {
                return Err(ConfigurationError::DuplicateRepositoryTag {
                    repository: id.as_str().to_owned(),
                    tag: tag.as_str().to_owned(),
                });
            }
        }
        Ok(Self { id, path, tags })
    }

    pub fn id(&self) -> &RepositoryId {
        &self.id
    }

    pub fn path(&self) -> &AbsolutePath {
        &self.path
    }

    pub fn tags(&self) -> &[RepositoryTag] {
        &self.tags
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceReference {
    id: SourceId,
    location: SourceLocation,
}

impl SourceReference {
    pub const fn new(id: SourceId, location: SourceLocation) -> Self {
        Self { id, location }
    }

    pub fn id(&self) -> &SourceId {
        &self.id
    }

    pub fn location(&self) -> &SourceLocation {
        &self.location
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AgentKind {
    Codex,
    ClaudeCode,
    Pi,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairControls {
    priority: Vec<AgentKind>,
    max_attempts: u8,
}

impl RepairControls {
    /// Build operational recovery policy without selecting a fallback order.
    /// The vector is the exact machine-declared priority.  An empty vector is
    /// valid and means that repair is unavailable.
    pub fn new(priority: Vec<AgentKind>, max_attempts: u8) -> Result<Self, ConfigurationError> {
        if max_attempts > MAX_REPAIR_ATTEMPTS {
            return Err(ConfigurationError::RepairAttemptsTooHigh {
                actual: max_attempts,
                maximum: MAX_REPAIR_ATTEMPTS,
            });
        }
        for (index, agent) in priority.iter().enumerate() {
            if priority[..index].contains(agent) {
                return Err(ConfigurationError::DuplicateRepairAgent { agent: *agent });
            }
        }
        Ok(Self {
            priority,
            max_attempts,
        })
    }

    pub fn priority(&self) -> &[AgentKind] {
        &self.priority
    }

    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }
}

impl Default for RepairControls {
    fn default() -> Self {
        Self {
            priority: Vec::new(),
            max_attempts: DEFAULT_REPAIR_ATTEMPTS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachineConcurrency {
    max_repositories: u16,
    max_child_work: u16,
}

impl MachineConcurrency {
    pub fn new(max_repositories: u16, max_child_work: u16) -> Result<Self, ConfigurationError> {
        validate_limit("max_repositories", max_repositories, MAX_REPOSITORIES)?;
        validate_limit("max_child_work", max_child_work, MAX_CHILD_WORK)?;
        Ok(Self {
            max_repositories,
            max_child_work,
        })
    }

    pub const fn max_repositories(self) -> u16 {
        self.max_repositories
    }

    pub const fn max_child_work(self) -> u16 {
        self.max_child_work
    }
}

impl Default for MachineConcurrency {
    fn default() -> Self {
        Self {
            max_repositories: DEFAULT_MAX_REPOSITORIES,
            max_child_work: DEFAULT_MAX_CHILD_WORK,
        }
    }
}

fn validate_limit(field: &'static str, value: u16, maximum: u16) -> Result<(), ConfigurationError> {
    if (1..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(ConfigurationError::InvalidConcurrency {
            field,
            value,
            maximum,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineConfiguration {
    version: SchemaVersion,
    repositories: Vec<DestinationRepository>,
    sources: Vec<SourceReference>,
    cache_root: Option<AbsolutePath>,
    concurrency: MachineConcurrency,
    repair: RepairControls,
}

impl MachineConfiguration {
    /// Construct an immutable machine authority snapshot.
    ///
    /// `cache_root` is optional for local-only source sets.  It is required
    /// whenever a remote source is present because remote materialization must
    /// stay below a machine-owned cache root.  Duplicate repository paths and
    /// source locations are checked by exact lexical value only; filesystem
    /// canonical identity is validated by the authority adapter.
    pub fn new(
        version: SchemaVersion,
        repositories: Vec<DestinationRepository>,
        sources: Vec<SourceReference>,
        cache_root: Option<AbsolutePath>,
        concurrency: MachineConcurrency,
        repair: RepairControls,
    ) -> Result<Self, ConfigurationError> {
        if cache_root.is_none()
            && sources
                .iter()
                .any(|source| matches!(source.location(), SourceLocation::Remote(_)))
        {
            return Err(ConfigurationError::MissingCacheRootForRemoteSource);
        }

        for (index, repository) in repositories.iter().enumerate() {
            if let Some(previous) = repositories[..index]
                .iter()
                .find(|previous| previous.id() == repository.id())
            {
                return Err(ConfigurationError::DuplicateRepositoryId {
                    id: repository.id().as_str().to_owned(),
                    first_path: previous.path().as_str().to_owned(),
                    second_path: repository.path().as_str().to_owned(),
                });
            }
            if repositories[..index]
                .iter()
                .any(|previous| previous.path() == repository.path())
            {
                return Err(ConfigurationError::DuplicateRepositoryPath {
                    path: repository.path().as_str().to_owned(),
                });
            }
        }
        for (index, source) in sources.iter().enumerate() {
            if sources[..index]
                .iter()
                .any(|previous| previous.id() == source.id())
            {
                return Err(ConfigurationError::DuplicateSourceId {
                    id: source.id().as_str().to_owned(),
                });
            }
            if let Some(previous) = sources[..index]
                .iter()
                .find(|previous| previous.location() == source.location())
            {
                return Err(ConfigurationError::DuplicateSourceLocation {
                    first_id: previous.id().as_str().to_owned(),
                    second_id: source.id().as_str().to_owned(),
                });
            }
        }
        Ok(Self {
            version,
            repositories,
            sources,
            cache_root,
            concurrency,
            repair,
        })
    }

    pub const fn version(&self) -> SchemaVersion {
        self.version
    }

    pub fn repositories(&self) -> &[DestinationRepository] {
        &self.repositories
    }

    pub fn sources(&self) -> &[SourceReference] {
        &self.sources
    }

    pub fn cache_root(&self) -> Option<&AbsolutePath> {
        self.cache_root.as_ref()
    }

    pub const fn concurrency(&self) -> MachineConcurrency {
        self.concurrency
    }

    pub fn repair(&self) -> &RepairControls {
        &self.repair
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigurationError {
    UnsupportedSchemaVersion {
        expected: u16,
        actual: u16,
    },
    InvalidSlug {
        field: &'static str,
        value: String,
    },
    InvalidIdentityText {
        field: &'static str,
        value: String,
    },
    InvalidPath {
        field: &'static str,
        reason: &'static str,
    },
    InvalidSourceLocation {
        reason: &'static str,
    },
    MissingCacheRootForRemoteSource,
    InvalidConcurrency {
        field: &'static str,
        value: u16,
        maximum: u16,
    },
    DuplicateRepairAgent {
        agent: AgentKind,
    },
    RepairAttemptsTooHigh {
        actual: u8,
        maximum: u8,
    },
    DuplicateRepositoryId {
        id: String,
        first_path: String,
        second_path: String,
    },
    DuplicateRepositoryPath {
        path: String,
    },
    DuplicateRepositoryTag {
        repository: String,
        tag: String,
    },
    DuplicateSourceId {
        id: String,
    },
    DuplicateSourceLocation {
        first_id: String,
        second_id: String,
    },
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { expected, actual } => {
                write!(
                    formatter,
                    "unsupported configuration schema version {actual}; expected {expected}"
                )
            }
            Self::InvalidSlug { field, value } => write!(
                formatter,
                "invalid {field} {value:?}; use lowercase ASCII letters, digits, '.', '_', or '-'"
            ),
            Self::InvalidIdentityText { field, value } => {
                write!(
                    formatter,
                    "invalid {field} {value:?}; control characters are not allowed"
                )
            }
            Self::InvalidPath { field, reason } => write!(formatter, "invalid {field}: {reason}"),
            Self::InvalidSourceLocation { reason } => {
                write!(formatter, "invalid source location: {reason}")
            }
            Self::MissingCacheRootForRemoteSource => {
                write!(formatter, "remote sources require a machine cache root")
            }
            Self::InvalidConcurrency {
                field,
                value,
                maximum,
            } => write!(
                formatter,
                "invalid {field}={value}; expected an integer in 1..={maximum}"
            ),
            Self::DuplicateRepairAgent { agent } => {
                write!(formatter, "duplicate repair agent {agent:?}")
            }
            Self::RepairAttemptsTooHigh { actual, maximum } => write!(
                formatter,
                "repair-attempt ceiling {actual} exceeds maximum {maximum}"
            ),
            Self::DuplicateRepositoryId {
                id,
                first_path,
                second_path,
            } => write!(
                formatter,
                "duplicate repository id {id:?} at {first_path:?} and {second_path:?}"
            ),
            Self::DuplicateRepositoryPath { path } => {
                write!(formatter, "duplicate destination repository path {path:?}")
            }
            Self::DuplicateRepositoryTag { repository, tag } => {
                write!(
                    formatter,
                    "duplicate tag {tag:?} on repository {repository:?}"
                )
            }
            Self::DuplicateSourceId { id } => write!(formatter, "duplicate source id {id:?}"),
            Self::DuplicateSourceLocation {
                first_id,
                second_id,
            } => write!(
                formatter,
                "source ids {first_id:?} and {second_id:?} use the same location"
            ),
        }
    }
}

impl Error for ConfigurationError {}
