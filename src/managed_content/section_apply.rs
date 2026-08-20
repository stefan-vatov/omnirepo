//! Grouped section application: every managed section for one
//! destination file, applied in one pass over the original bytes.
//!
//! Existing sections are replaced in place; absent sections are appended
//! in write order, each preceded by one separating blank line in the
//! file's detectable LF or CRLF style (LF when no style is detectable).
//! Content outside managed sections — including sections not written
//! this run — is preserved byte-exact.  Payload bytes are authoritative
//! and never normalized; a payload line that resembles a marker is
//! invalid, never escaped (canon/architecture/managed-content.md).

#![allow(dead_code)]

use super::delimiters::{DelimiterSyntax, LineClass};
use super::partial_scan::{ScanOutcome, logical_line, scan_sections, split_inclusive_lines};
use crate::configuration::SectionId;
use std::{error::Error, fmt};

/// One section write: the ID and its authoritative payload bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionWrite {
    pub id: SectionId,
    pub payload: Vec<u8>,
}

/// The outcome for one written section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionOutcome {
    pub id: SectionId,
    /// The section existed in the original content (replaced in place);
    /// false means it was appended.
    pub existed: bool,
    /// The written block differs from the original block bytes.
    pub changed: bool,
}

/// The applied file: the complete new content and per-section outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedFile {
    pub content: Vec<u8>,
    /// True when the content differs from the original bytes.
    pub changed: bool,
    pub sections: Vec<SectionOutcome>,
}

/// Application failures; the file is never touched on failure.
#[derive(Debug)]
pub enum ApplyError {
    /// The destination's existing topology is ambiguous.
    Topology { reason: String },
    /// A payload line resembles a marker line for this format.
    PayloadMarker { id: SectionId, line: u64 },
    /// The same section ID is written twice in one group.
    DuplicateWrite { id: SectionId },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Topology { reason } => {
                write!(formatter, "ambiguous delimiter topology: {reason}")
            }
            Self::PayloadMarker { id, line } => write!(
                formatter,
                "the payload for section {id} carries a marker-like line at line {line}; payload-like markers are invalid"
            ),
            Self::DuplicateWrite { id } => {
                write!(formatter, "section {id} is written twice in one group")
            }
        }
    }
}
impl Error for ApplyError {}

/// Apply every section write to the original content in one pass.
pub fn apply_sections(
    original: &[u8],
    syntax: &DelimiterSyntax,
    writes: &[SectionWrite],
) -> Result<AppliedFile, ApplyError> {
    for (index, write) in writes.iter().enumerate() {
        if writes[..index].iter().any(|prior| prior.id == write.id) {
            return Err(ApplyError::DuplicateWrite {
                id: write.id.clone(),
            });
        }
        check_payload(write, syntax)?;
    }
    let existing = match scan_sections(original, syntax) {
        ScanOutcome::Sections(sections) => sections,
        ScanOutcome::Invalid { reason } => return Err(ApplyError::Topology { reason }),
    };
    let eol = detect_eol(original);
    let chunks = split_inclusive_lines(original);
    let mut content: Vec<u8> = Vec::with_capacity(original.len());
    let mut outcomes: Vec<SectionOutcome> = Vec::new();

    // Replace existing sections in place; preserve everything else exactly.
    let mut line_number: u64 = 0;
    let mut skip_through: u64 = 0;
    for chunk in &chunks {
        line_number += 1;
        if line_number <= skip_through {
            continue;
        }
        let replaced = existing
            .iter()
            .find(|section| section.bounds.start_line == line_number)
            .and_then(|section| {
                writes
                    .iter()
                    .find(|write| write.id == section.id)
                    .map(|write| (section, write))
            });
        let Some((section, write)) = replaced else {
            content.extend_from_slice(chunk);
            continue;
        };
        let block = section_block(write, syntax, eol);
        let old_block = lines_bytes(&chunks, section.bounds.start_line, section.bounds.end_line);
        outcomes.push(SectionOutcome {
            id: write.id.clone(),
            existed: true,
            changed: old_block != block.as_slice(),
        });
        content.extend_from_slice(&block);
        skip_through = section.bounds.end_line;
    }

    // Append absent sections in write order, each after one blank line.
    for write in writes {
        if existing.iter().any(|section| section.id == write.id) {
            continue;
        }
        if !content.is_empty() {
            if !content.ends_with(b"\n") {
                content.extend_from_slice(eol);
            }
            if !(content.ends_with(b"\n\n") || content.ends_with(b"\r\n\r\n")) {
                content.extend_from_slice(eol);
            }
        }
        content.extend_from_slice(&section_block(write, syntax, eol));
        outcomes.push(SectionOutcome {
            id: write.id.clone(),
            existed: false,
            changed: true,
        });
    }

    let changed = content != original;
    Ok(AppliedFile {
        content,
        changed,
        sections: outcomes,
    })
}

/// The canonical block for one section: open marker, the exact payload
/// bytes (with one terminator added only when a nonempty payload lacks
/// its own), then the close marker.  Marker lines are omnirepo-owned and
/// use the file's newline style.
fn section_block(write: &SectionWrite, syntax: &DelimiterSyntax, eol: &[u8]) -> Vec<u8> {
    let mut block = Vec::with_capacity(write.payload.len() + 64);
    block.extend_from_slice(syntax.open_marker(&write.id).as_bytes());
    block.extend_from_slice(eol);
    if !write.payload.is_empty() {
        block.extend_from_slice(&write.payload);
        if !write.payload.ends_with(b"\n") {
            block.extend_from_slice(eol);
        }
    }
    block.extend_from_slice(syntax.close_marker(&write.id).as_bytes());
    block.extend_from_slice(eol);
    block
}

/// A payload line that resembles a marker for this format is invalid.
fn check_payload(write: &SectionWrite, syntax: &DelimiterSyntax) -> Result<(), ApplyError> {
    for (index, chunk) in split_inclusive_lines(&write.payload).iter().enumerate() {
        if !matches!(
            syntax.classify_line(logical_line(chunk)),
            LineClass::Content
        ) {
            return Err(ApplyError::PayloadMarker {
                id: write.id.clone(),
                line: (index + 1) as u64,
            });
        }
    }
    Ok(())
}

/// The exact original bytes of lines start..=end (1-based).
fn lines_bytes(chunks: &[&[u8]], start_line: u64, end_line: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in &chunks[(start_line - 1) as usize..end_line as usize] {
        bytes.extend_from_slice(chunk);
    }
    bytes
}

/// The file's newline style, detected from its first line terminator.
/// Payload bytes later in the file must not flip the style between runs:
/// detection from the first terminator is stable, because the first line
/// is either preserved local content or an omnirepo-owned marker already
/// written in the detected style.  LF when no style is detectable.
fn detect_eol(original: &[u8]) -> &'static [u8] {
    match original.iter().position(|byte| *byte == b'\n') {
        Some(index) if index > 0 && original[index - 1] == b'\r' => b"\r\n",
        _ => b"\n",
    }
}

#[cfg(test)]
mod section_apply_tests;
