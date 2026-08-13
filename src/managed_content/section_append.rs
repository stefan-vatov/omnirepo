//! Absent-section append content with canonical separators.
//!
//! A missing file appends the canonical section; a nonempty file gets a
//! canonical separator before the section; empty, no-final-newline, and
//! CRLF cases produce the exact expected content.  A repeated sync finds
//! exactly one pair and is a no-op: no duplicate section is ever appended.

#![allow(dead_code)]

use super::delimiters::DelimiterSyntax;
use super::partial_scan::{Topology, scan_partial};
use std::{error::Error, fmt};

/// Append failures.
#[derive(Debug)]
pub enum AppendError {
    SectionAlreadyPresent { reason: String },
}

impl fmt::Display for AppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SectionAlreadyPresent { reason } => {
                write!(formatter, "a managed section is already present: {reason}")
            }
        }
    }
}
impl Error for AppendError {}

/// Build the exact append content for an absent section.  The existing
/// content is preserved verbatim; a canonical separator (a blank line)
/// is inserted only when the file is nonempty and lacks a trailing
/// newline separation.  Returns None when no append is needed (the file
/// is empty and the payload is empty).
pub fn build_absent_section_append(
    existing: &str,
    syntax: &DelimiterSyntax,
    payload: &str,
) -> Result<Option<String>, AppendError> {
    match scan_partial(existing, syntax) {
        Topology::Absent => {}
        Topology::ExactlyOne { .. } | Topology::Ambiguous { .. } => {
            return Err(AppendError::SectionAlreadyPresent {
                reason: "the file already carries managed section markers".to_owned(),
            });
        }
    }
    if existing.is_empty() && payload.is_empty() {
        return Ok(None);
    }
    let mut content = String::new();
    if existing.is_empty() {
        content.push_str(syntax.open);
        content.push('\n');
        if !payload.is_empty() {
            content.push_str(payload);
            content.push('\n');
        }
        content.push_str(syntax.close);
        content.push('\n');
        return Ok(Some(content));
    }
    // Nonempty file: preserve it verbatim, then the canonical separator
    // (one blank line in the file's own line style) unless the file
    // already ends with a blank line.
    content.push_str(existing);
    let crlf = existing.contains("\r\n");
    let separator = if crlf { "\r\n" } else { "\n" };
    let ends_blank = existing.ends_with("\n\n") || existing.ends_with("\r\n\r\n");
    if !ends_blank {
        // A blank separator line: one terminator for the last line and one
        // for the blank line when the file lacks a final newline.
        if !existing.ends_with('\n') {
            content.push_str(separator);
        }
        content.push_str(separator);
    }
    content.push_str(syntax.open);
    content.push('\n');
    if !payload.is_empty() {
        content.push_str(payload);
        content.push('\n');
    }
    content.push_str(syntax.close);
    content.push('\n');
    Ok(Some(content))
}

#[cfg(test)]
mod section_append_tests;
