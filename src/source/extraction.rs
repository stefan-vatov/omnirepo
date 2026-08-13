//! Exact whole-file and section payload extraction with provenance.
//!
//! A declaration content locator resolves to one payload representation:
//! either the whole file's exact bytes (BOM, newlines, and every byte
//! preserved) or a decided line section (1-based, inclusive) with its
//! section identity.  Missing, non-regular, escaping, and ambiguous
//! locators fail contextually; provenance records the locator and a
//! deterministic content identity.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The requested payload kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayloadKind {
    /// The whole file, byte-exact.
    WholeFile,
    /// A decided line section (1-based, inclusive).
    Section { start_line: u64, end_line: u64 },
}

/// One extracted payload with its provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedPayload {
    pub locator: String,
    pub section: Option<(u64, u64)>,
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
    /// The section decision is invalid.
    Section { locator: String, reason: String },
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
            Self::Section { locator, reason } => {
                write!(
                    formatter,
                    "locator {locator:?} section decision is invalid: {reason}"
                )
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
            section: None,
            bytes: content.to_vec(),
            content_identity: content_identity(content),
        }),
        PayloadKind::Section {
            start_line,
            end_line,
        } => {
            let start_line = *start_line;
            let end_line = *end_line;
            if start_line == 0 || end_line == 0 || start_line > end_line {
                return Err(ExtractionError::Section {
                    locator: locator.to_owned(),
                    reason: format!("invalid section {start_line}..={end_line}"),
                });
            }
            let lines = split_lines(content);
            if end_line > lines.len() as u64 {
                return Err(ExtractionError::Section {
                    locator: locator.to_owned(),
                    reason: format!(
                        "section ends at line {end_line} but the content has {} lines",
                        lines.len()
                    ),
                });
            }
            let mut bytes = Vec::new();
            for line in &lines[(start_line - 1) as usize..end_line as usize] {
                bytes.extend_from_slice(line);
            }
            let identity = content_identity(&bytes);
            Ok(ExtractedPayload {
                locator: locator.to_owned(),
                section: Some((start_line, end_line)),
                bytes,
                content_identity: identity,
            })
        }
    }
}

/// Split content into lines preserving every byte (including terminators).
fn split_lines(content: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in content.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&content[start..=index]);
            start = index + 1;
        }
    }
    if start < content.len() {
        lines.push(&content[start..]);
    }
    if lines.is_empty() {
        lines.push(&[]);
    }
    lines
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
