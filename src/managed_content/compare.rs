//! Exact compare and unchanged no-op detection for managed targets.
//!
//! The owner-decided comparison unit is exact bytes: encoding, BOM, newline
//! style, trailing whitespace, permissions, and timestamps are never
//! normalized.  Equal content is a true no-op (no temp creation, write,
//! rename, metadata, or Git-visible mutation).  A difference — including an
//! absent target — yields one prepared replacement operation.  The caller
//! reads the current target through the platform authority (containment-safe,
//! no-follow) and passes the bytes; a read failure happens before compare and
//! preserves the target untouched.  This module stays dependency-free per the
//! frozen module map (`managed_content` may only depend on `configuration`).

#![allow(dead_code)]

use super::transaction::{ParentDirectories, PlanError, TransactionPlan};
use std::{error::Error, fmt, path::PathBuf};

/// Compare outcome for one managed target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompareOutcome {
    /// The target bytes equal the authoritative bytes: no effect of any kind.
    Unchanged,
    /// The target differs or is absent: one prepared replacement operation.
    Replacement(TransactionPlan),
}

/// Typed compare failures; the target is always preserved.
#[derive(Debug)]
pub enum CompareError {
    /// The prepared replacement plan is invalid (path or identity contract).
    Plan(PlanError),
}

impl fmt::Display for CompareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => write!(formatter, "cannot prepare replacement: {error}"),
        }
    }
}
impl Error for CompareError {}

/// Compare the authoritative bytes against the current target bytes.
///
/// `current` is `None` when the target is absent (a create replacement whose
/// parents must be created).  Side effects: none — equality never touches
/// temp files, metadata, or Git; differences return a prepared plan without
/// executing it.
pub fn compare(
    operation_id: &str,
    relative: &str,
    current: Option<&[u8]>,
    authoritative: &[u8],
) -> Result<CompareOutcome, CompareError> {
    match current {
        None => {
            let parents = parent_components(relative)?;
            let plan = TransactionPlan::new(
                operation_id,
                PathBuf::from(relative),
                ParentDirectories::created(parents),
            )
            .map_err(CompareError::Plan)?;
            Ok(CompareOutcome::Replacement(plan))
        }
        Some(bytes) if bytes == authoritative => Ok(CompareOutcome::Unchanged),
        Some(_) => {
            let plan = TransactionPlan::new(
                operation_id,
                PathBuf::from(relative),
                ParentDirectories::Existing,
            )
            .map_err(CompareError::Plan)?;
            Ok(CompareOutcome::Replacement(plan))
        }
    }
}

fn parent_components(relative: &str) -> Result<Vec<PathBuf>, CompareError> {
    let components: Vec<&str> = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let mut parents = Vec::new();
    let mut current = PathBuf::new();
    for part in &components[..components.len().saturating_sub(1)] {
        current.push(part);
        parents.push(current.clone());
    }
    Ok(parents)
}
