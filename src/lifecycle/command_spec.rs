//! Repository command snapshot translation into bounded process specs.
//!
//! The owner-decided command representation (order, cwd, environment,
//! timeout, stdin, output capture, cancellation) becomes immutable
//! executable specs bound to the repository and plan identity.  Absent,
//! empty, and duplicate commands follow policy; specs carry a canonical
//! cwd and a sanitized environment; no shell is introduced unless the spec
//! explicitly selects one.

#![allow(dead_code)]

use crate::platform::RelativePath;
use std::{error::Error, fmt, path::PathBuf, time::Duration};

/// One translated command spec.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub repository: String,
    pub plan_identity: String,
    pub position: usize,
    pub argv: Vec<String>,
    /// Canonical working directory (root-relative).
    pub cwd: RelativePath,
    /// Sanitized environment: only the declared variables.
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub stdin: Option<String>,
    pub capture_output: bool,
    /// A shell is introduced only when explicitly selected.
    pub shell: Option<String>,
}

/// The declared command shape (owner-decided representation).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredCommand {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Option<Duration>,
    pub stdin: Option<String>,
    pub capture_output: bool,
    pub shell: Option<String>,
}

/// Translation failures.
#[derive(Debug)]
pub enum SpecError {
    EmptyCommand {
        repository: String,
    },
    DuplicatePosition {
        position: usize,
    },
    InvalidCwd {
        repository: String,
        cwd: String,
        reason: String,
    },
    EmptyArgv {
        repository: String,
    },
}

impl fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCommand { repository } => {
                write!(formatter, "repository {repository} declares no commands")
            }
            Self::DuplicatePosition { position } => {
                write!(formatter, "command position {position} is duplicated")
            }
            Self::InvalidCwd {
                repository,
                cwd,
                reason,
            } => write!(
                formatter,
                "repository {repository} command cwd {cwd:?} is invalid: {reason}"
            ),
            Self::EmptyArgv { repository } => {
                write!(
                    formatter,
                    "repository {repository} has a command with no argv"
                )
            }
        }
    }
}
impl Error for SpecError {}

/// Translate the declared commands into immutable specs.
///
/// Absent or empty command lists fail typed; an ordered list must not
/// duplicate positions; each argv must be non-empty; the cwd defaults to
/// the repository root and must be a valid root-relative path; the
/// environment is copied exactly (sanitized by construction, no ambient
/// values); the default timeout is applied when none is declared.
pub fn translate_commands(
    repository: &str,
    plan_identity: &str,
    declared: &[DeclaredCommand],
    default_timeout: Duration,
) -> Result<Vec<CommandSpec>, SpecError> {
    if declared.is_empty() {
        return Err(SpecError::EmptyCommand {
            repository: repository.to_owned(),
        });
    }
    let mut seen_positions = Vec::new();
    let mut specs = Vec::with_capacity(declared.len());
    for (position, command) in declared.iter().enumerate() {
        if seen_positions.contains(&position) {
            return Err(SpecError::DuplicatePosition { position });
        }
        seen_positions.push(position);
        if command.argv.is_empty() {
            return Err(SpecError::EmptyArgv {
                repository: repository.to_owned(),
            });
        }
        let cwd = match &command.cwd {
            Some(cwd) => RelativePath::parse(cwd).map_err(|error| SpecError::InvalidCwd {
                repository: repository.to_owned(),
                cwd: cwd.clone(),
                reason: error.to_string(),
            })?,
            None => RelativePath::root(),
        };
        specs.push(CommandSpec {
            repository: repository.to_owned(),
            plan_identity: plan_identity.to_owned(),
            position,
            argv: command.argv.clone(),
            cwd,
            env: command.env.clone(),
            timeout: command.timeout.unwrap_or(default_timeout),
            stdin: command.stdin.clone(),
            capture_output: command.capture_output,
            shell: command.shell.clone(),
        });
    }
    Ok(specs)
}

/// The canonical cwd for a spec (repository root joined with the spec
/// cwd), used by the executor.
pub fn canonical_cwd(repository_root: &std::path::Path, spec: &CommandSpec) -> PathBuf {
    if spec.cwd.components().next().is_none() {
        return repository_root.to_path_buf();
    }
    repository_root.join(spec.cwd.display())
}

#[cfg(test)]
mod command_spec_tests;
