//! Exact whole-file payload extraction with provenance.
//!
//! A declaration content locator resolves to the whole file's exact bytes
//! (BOM, newlines, and every byte preserved).  Missing, non-regular,
//! escaping, and ambiguous locators fail contextually; provenance records
//! the locator and a deterministic content identity.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The requested payload kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayloadKind {
    /// The whole file, byte-exact.
    WholeFile,
}

/// One extracted payload with its provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedPayload {
    pub locator: String,
    pub bytes: Vec<u8>,
    /// Deterministic content identity (length + FNV-1a 64).
    pub content_identity: String,
}

/// Extraction failures.
#[derive(Debug)]
pub enum ExtractionError {
    /// The locator escapes its root or is otherwise unusable.
    Escaping { locator: String, reason: String },
    /// The locator is ambiguous (empty, NUL, or unresolved).
    Ambiguous { locator: String, reason: String },
}

impl fmt::Display for ExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Escaping { locator, reason } => {
                write!(formatter, "locator {locator:?} escapes its root: {reason}")
            }
            Self::Ambiguous { locator, reason } => {
                write!(formatter, "locator {locator:?} is ambiguous: {reason}")
            }
        }
    }
}
impl Error for ExtractionError {}

/// Validate a locator string for root-relative use (the reader resolves it
/// through the typed authority; this rejects escaping and ambiguous forms
/// before any resolution).
pub fn validate_locator(locator: &str) -> Result<(), ExtractionError> {
    if locator.is_empty() {
        return Err(ExtractionError::Ambiguous {
            locator: locator.to_owned(),
            reason: "the locator is empty".to_owned(),
        });
    }
    if locator.contains('\0') {
        return Err(ExtractionError::Ambiguous {
            locator: locator.to_owned(),
            reason: "the locator carries a NUL byte".to_owned(),
        });
    }
    if locator.starts_with('/') {
        return Err(ExtractionError::Escaping {
            locator: locator.to_owned(),
            reason: "the locator is absolute".to_owned(),
        });
    }
    for component in locator.split('/') {
        if component.is_empty() {
            return Err(ExtractionError::Ambiguous {
                locator: locator.to_owned(),
                reason: "the locator has an empty path component".to_owned(),
            });
        }
        if component == ".." {
            return Err(ExtractionError::Escaping {
                locator: locator.to_owned(),
                reason: "the locator ascends above its root".to_owned(),
            });
        }
    }
    Ok(())
}

/// Extract the payload from the exact content bytes.
pub fn extract_payload(
    locator: &str,
    content: &[u8],
    kind: &PayloadKind,
) -> Result<ExtractedPayload, ExtractionError> {
    validate_locator(locator)?;
    match kind {
        PayloadKind::WholeFile => Ok(ExtractedPayload {
            locator: locator.to_owned(),
            bytes: content.to_vec(),
            content_identity: content_identity(content),
        }),
    }
}

/// Deterministic content identity: length plus FNV-1a 64 over the bytes.
pub fn content_identity(content: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in content {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}:{}", content.len(), hash)
}
