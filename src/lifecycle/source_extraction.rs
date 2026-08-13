//! Authority-typed payload extraction (the source snapshot seam).
//!
//! Each declaration content locator is resolved through the typed read-only
//! source authority (no-follow, regular-file targets only) and the exact
//! bytes are handed to the pure extractor.  Missing, aliased, linked,
//! non-regular, or unreadable targets fail contextually before any
//! extraction.

#![allow(dead_code)]

use crate::platform::{AuthorityRoot, ObjectClass, ReadOnly, RelativePath, SourceSnapshotRoot};
use crate::source::{ExtractedPayload, ExtractionError, PayloadKind, extract_payload};

/// Extract one payload for a locator under the typed snapshot root.
pub fn extract_from_snapshot(
    root: &AuthorityRoot<SourceSnapshotRoot, ReadOnly>,
    locator: &str,
    kind: &PayloadKind,
) -> Result<ExtractedPayload, ExtractionError> {
    crate::source::validate_locator(locator)?;
    let relative = RelativePath::parse(locator).map_err(|error| ExtractionError::Escaping {
        locator: locator.to_owned(),
        reason: error.to_string(),
    })?;
    let target = root
        .resolve_read(&relative, ObjectClass::RegularFile)
        .map_err(|error| ExtractionError::Ambiguous {
            locator: locator.to_owned(),
            reason: format!("unreadable through the source authority: {error}"),
        })?;
    let mut handle = target
        .try_clone_file()
        .map_err(|error| ExtractionError::Ambiguous {
            locator: locator.to_owned(),
            reason: format!("cannot clone the read handle: {error}"),
        })?;
    let mut bytes = Vec::new();
    use std::io::Read;
    handle
        .read_to_end(&mut bytes)
        .map_err(|error| ExtractionError::Ambiguous {
            locator: locator.to_owned(),
            reason: format!("cannot read the content: {error}"),
        })?;
    extract_payload(locator, &bytes, kind)
}

#[cfg(test)]
mod source_extraction_tests;
