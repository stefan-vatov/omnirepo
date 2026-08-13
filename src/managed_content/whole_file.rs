//! Whole-file outcome classification.
//!
//! Missing targets create, equal targets are unchanged (true no-op),
//! different targets replace (local drift is overwritten without prompt),
//! and the empty, nested, read-only, invalid-encoding, and
//! acquisition-failure cases return typed outcomes.  A source
//! acquisition failure mutates nothing: classification is pure.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The decided whole-file outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WholeFileOutcome {
    /// The target is missing; create it.
    Create,
    /// The target exists with equal bytes; true no-op.
    Unchanged,
    /// The target differs; replace it (local drift is overwritten without
    /// prompt).
    Replace,
    /// The payload is empty; the decided behavior is a typed create.
    EmptyCreate,
}

/// Typed failure outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WholeFileFailure {
    /// The target path is nested inside a managed section.
    Nested { reason: String },
    /// The target is not writable.
    ReadOnly { reason: String },
    /// The payload is not valid for the target's encoding.
    InvalidEncoding { reason: String },
    /// The source acquisition failed; nothing is mutated.
    SourceUnavailable { reason: String },
}

impl fmt::Display for WholeFileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nested { reason } => write!(formatter, "nested target: {reason}"),
            Self::ReadOnly { reason } => write!(formatter, "read-only target: {reason}"),
            Self::InvalidEncoding { reason } => {
                write!(formatter, "invalid payload encoding: {reason}")
            }
            Self::SourceUnavailable { reason } => {
                write!(formatter, "source unavailable: {reason}")
            }
        }
    }
}
impl Error for WholeFileFailure {}

/// Classify the whole-file outcome from the pure inputs.  The caller
/// resolves the file state through the authority; classification never
/// touches the filesystem and never mutates anything.
pub fn classify_whole_file(
    target_exists: bool,
    target_bytes: Option<&[u8]>,
    payload_bytes: &[u8],
) -> Result<WholeFileOutcome, WholeFileFailure> {
    if payload_bytes.is_empty() {
        return Ok(WholeFileOutcome::EmptyCreate);
    }
    match (target_exists, target_bytes) {
        (false, _) => Ok(WholeFileOutcome::Create),
        (true, Some(existing)) if existing == payload_bytes => Ok(WholeFileOutcome::Unchanged),
        (true, Some(_)) => Ok(WholeFileOutcome::Replace),
        (true, None) => Err(WholeFileFailure::ReadOnly {
            reason: "the target exists but its content cannot be read".to_owned(),
        }),
    }
}

#[cfg(test)]
mod whole_file_tests;
