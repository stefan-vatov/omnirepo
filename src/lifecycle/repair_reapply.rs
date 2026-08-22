//! Reapply authoritative synchronization and rerun the frozen verification
//! after a successful repair.
//!
//! After the post-agent delta is the expected repair effect, the
//! authoritative content is re-delivered into the managed file: a whole
//! file is rewritten byte-exact; a named section is spliced in place,
//! preserving every other byte of the file — including other managed
//! sections.  The frozen verification is then rerun: the managed payload
//! must be byte-identical to the authoritative content while the frozen
//! repair inputs are still present.

#![allow(dead_code)]

#[cfg(test)]
mod repair_reapply_tests;

use crate::configuration::SectionId;
use crate::managed_content::{
    DelimiterError, DelimiterSyntax, Representation, ScanOutcome, SectionWrite, apply_sections,
    check_exact_representation, lookup_by_extension, scan_sections, split_inclusive_lines,
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
        reason: String,
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
            Self::MalformedSection { path, reason } => {
                write!(
                    formatter,
                    "the managed sections in {} are malformed: {reason}",
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
/// A whole file (`section` is None) is rewritten byte-exact.  A named
/// section is spliced in place through the grouped section engine; every
/// byte outside that section is preserved.  A representation that cannot
/// carry the bytes exactly fails before any write.
pub fn reapply_authoritative(
    working: &Path,
    managed: &str,
    section: Option<&SectionId>,
    authoritative: &[u8],
) -> Result<(), ReapplyError> {
    let (_base, target) = managed_target(working, managed)?;
    match check_exact_representation(authoritative, true) {
        Representation::Exact => {}
        other => return Err(ReapplyError::Representation(other)),
    }
    let rendered = match section {
        None => authoritative.to_vec(),
        Some(section) => {
            let syntax = syntax_for(managed)?;
            let current = fs::read(&target).map_err(|error| ReapplyError::Read {
                path: target.clone(),
                reason: error.to_string(),
            })?;
            let applied = apply_sections(
                &current,
                syntax,
                &[SectionWrite {
                    id: section.clone(),
                    payload: authoritative.to_vec(),
                }],
            )
            .map_err(|error| ReapplyError::MalformedSection {
                path: target.clone(),
                reason: error.to_string(),
            })?;
            // An unchanged target receives no filesystem write.
            if !applied.changed {
                return Ok(());
            }
            applied.content
        }
    };
    // Atomic and durable replacement (canon/architecture/managed-content.md):
    // a same-directory temporary, sync, rename, and parent-directory sync,
    // so an interrupted reapply exposes only the old or the new complete
    // file.
    crate::lifecycle::replace::replace_bytes_atomically(
        working,
        managed,
        "repair-reapply",
        &rendered,
    )
    .map_err(|error| ReapplyError::Write {
        path: target.clone(),
        reason: error.to_string(),
    })?;
    Ok(())
}

/// Rerun the frozen verification: the managed payload must be
/// byte-identical to the authoritative content, and the frozen repair
/// inputs must still be present.
pub fn rerun_frozen_verification(
    working: &Path,
    managed: &str,
    section: Option<&SectionId>,
    authoritative: &[u8],
    frozen_inputs: &[String],
) -> Result<ReapplyVerdict, ReapplyError> {
    if frozen_inputs.is_empty() {
        return Err(ReapplyError::FrozenInputsMissing);
    }
    let (_base, target) = managed_target(working, managed)?;
    let bytes = fs::read(&target).map_err(|error| ReapplyError::Read {
        path: target.clone(),
        reason: error.to_string(),
    })?;
    let Some(section) = section else {
        return Ok(if bytes == authoritative {
            ReapplyVerdict::Verified
        } else {
            ReapplyVerdict::Drifted
        });
    };
    let syntax = syntax_for(managed)?;
    // One scan; the verdict compares the section body bytes directly, so
    // marker-line details (a missing final terminator, the file's newline
    // style) never mask a byte-identical payload.
    let sections = match scan_sections(&bytes, syntax) {
        ScanOutcome::Sections(sections) => sections,
        ScanOutcome::Invalid { .. } => return Ok(ReapplyVerdict::Drifted),
    };
    let Some(found) = sections.iter().find(|found| &found.id == section) else {
        return Ok(ReapplyVerdict::Drifted);
    };
    let chunks = split_inclusive_lines(&bytes);
    let body: Vec<u8> =
        chunks[found.bounds.start_line as usize..(found.bounds.end_line - 1) as usize].concat();
    // The written body is the exact payload, plus one terminator when a
    // nonempty payload lacks its own (the engine's block rule).
    let verified = body == authoritative
        || (!authoritative.is_empty()
            && !authoritative.ends_with(b"\n")
            && body
                .strip_suffix(b"\r\n")
                .or_else(|| body.strip_suffix(b"\n"))
                == Some(authoritative));
    Ok(if verified {
        ReapplyVerdict::Verified
    } else {
        ReapplyVerdict::Drifted
    })
}
