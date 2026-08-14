//! Reapply authoritative synchronization and rerun the frozen verification
//! after a successful repair.
//!
//! After the post-agent delta is the expected repair effect, the
//! authoritative content is re-delivered into the managed file (the
//! synchronization reapplication): the file is rebuilt as the canonical
//! managed section carrying the authoritative payload, byte-exact.  The
//! frozen verification is then rerun: the payload inside the managed
//! section must be byte-identical to the authoritative content while the
//! frozen repair inputs are still present.

#![allow(dead_code)]

#[cfg(test)]
mod repair_reapply_tests;

use crate::managed_content::{
    DelimiterError, DelimiterSyntax, Representation, check_exact_representation,
    lookup_by_extension,
};
use crate::platform::RelativePath;
use std::{error::Error, fmt, fs, path::Path};

/// The rerun verification verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReapplyVerdict {
    /// The managed payload is byte-identical to the authoritative content.
    Verified,
    /// The destination drifted from the authoritative content.
    Drifted,
}

/// Reapplication failures.
#[derive(Debug)]
pub enum ReapplyError {
    ManagedPath {
        reason: String,
    },
    FrozenInputsMissing,
    Delimiters(DelimiterError),
    Representation(Representation),
    Read {
        path: std::path::PathBuf,
        reason: String,
    },
    Write {
        path: std::path::PathBuf,
        reason: String,
    },
    MalformedSection {
        path: std::path::PathBuf,
    },
}

impl fmt::Display for ReapplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManagedPath { reason } => {
                write!(formatter, "reapply managed path failure: {reason}")
            }
            Self::FrozenInputsMissing => {
                write!(formatter, "the frozen repair inputs are missing")
            }
            Self::Delimiters(error) => write!(formatter, "reapply delimiter failure: {error}"),
            Self::Representation(error) => {
                write!(formatter, "reapply representation failure: {error}")
            }
            Self::Read { path, reason } => {
                write!(
                    formatter,
                    "reapply read failure {}: {reason}",
                    path.display()
                )
            }
            Self::Write { path, reason } => {
                write!(
                    formatter,
                    "reapply write failure {}: {reason}",
                    path.display()
                )
            }
            Self::MalformedSection { path } => {
                write!(
                    formatter,
                    "the managed section in {} is malformed",
                    path.display()
                )
            }
        }
    }
}
impl Error for ReapplyError {}

fn managed_target<'a>(
    working: &'a Path,
    managed: &str,
) -> Result<(&'a Path, std::path::PathBuf), ReapplyError> {
    let relative = RelativePath::parse(managed).map_err(|error| ReapplyError::ManagedPath {
        reason: error.to_string(),
    })?;
    Ok((working, working.join(relative.display())))
}

fn syntax_for(managed: &str) -> Result<&'static DelimiterSyntax, ReapplyError> {
    lookup_by_extension(managed).map_err(ReapplyError::Delimiters)
}

/// Reapply the authoritative content into the managed file.
///
/// The file is rebuilt as the canonical managed section carrying the
/// authoritative payload byte-exact.  A representation that cannot carry
/// the bytes exactly fails before any write.
pub fn reapply_authoritative(
    working: &Path,
    managed: &str,
    authoritative: &[u8],
) -> Result<(), ReapplyError> {
    let (_base, target) = managed_target(working, managed)?;
    let syntax = syntax_for(managed)?;
    match check_exact_representation(authoritative, true) {
        Representation::Exact => {}
        other => return Err(ReapplyError::Representation(other)),
    }
    let payload = std::str::from_utf8(authoritative).map_err(|error| {
        ReapplyError::Representation(Representation::Unsupported {
            reason: error.to_string(),
        })
    })?;
    let mut rendered = String::new();
    rendered.push_str(syntax.open);
    rendered.push('\n');
    rendered.push_str(payload);
    if !payload.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push_str(syntax.close);
    rendered.push('\n');
    fs::write(&target, rendered).map_err(|error| ReapplyError::Write {
        path: target.clone(),
        reason: error.to_string(),
    })?;
    Ok(())
}

/// Rerun the frozen verification: the payload inside the managed section
/// must be byte-identical to the authoritative content, and the frozen
/// repair inputs must still be present.
pub fn rerun_frozen_verification(
    working: &Path,
    managed: &str,
    authoritative: &[u8],
    frozen_inputs: &[String],
) -> Result<ReapplyVerdict, ReapplyError> {
    if frozen_inputs.is_empty() {
        return Err(ReapplyError::FrozenInputsMissing);
    }
    let (_base, target) = managed_target(working, managed)?;
    let syntax = syntax_for(managed)?;
    let bytes = fs::read(&target).map_err(|error| ReapplyError::Read {
        path: target.clone(),
        reason: error.to_string(),
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        ReapplyError::Representation(Representation::Unsupported {
            reason: error.to_string(),
        })
    })?;
    let start = match text.find(syntax.open) {
        Some(start) => start,
        None => return Ok(ReapplyVerdict::Drifted),
    };
    let after_start = start + syntax.open.len();
    let after_start = if text[after_start..].starts_with('\n') {
        after_start + 1
    } else {
        after_start
    };
    let rest = &text[after_start..];
    let end = match rest.find(syntax.close) {
        Some(offset) => after_start + offset,
        None => return Ok(ReapplyVerdict::Drifted),
    };
    let payload = &text[after_start..end];
    if payload.as_bytes() == authoritative {
        Ok(ReapplyVerdict::Verified)
    } else {
        Ok(ReapplyVerdict::Drifted)
    }
}
