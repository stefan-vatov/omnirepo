//! Exact existing-section replacement content building.
//!
//! The outside content stays byte-identical; empty and adjacent sections
//! work; marker-like payload text follows the escaping rules; an equal
//! body produces no transaction request (changed = false).

#![allow(dead_code)]

use super::delimiters::DelimiterSyntax;
use super::partial_scan::Bounds;
use std::{error::Error, fmt};

/// The built replacement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionReplacement {
    pub content: String,
    /// False when the body is unchanged: no transaction request is made.
    pub changed: bool,
}

/// Replacement failures.
#[derive(Debug)]
pub enum SectionError {
    BoundsOutside { reason: String },
}

impl fmt::Display for SectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BoundsOutside { reason } => {
                write!(formatter, "section bounds are invalid: {reason}")
            }
        }
    }
}
impl Error for SectionError {}

/// Escape marker-like payload text: any line that would look like an open
/// or close marker is prefixed so it cannot close or open the section.
pub fn escape_payload(payload: &str, syntax: &DelimiterSyntax) -> String {
    payload
        .lines()
        .map(|line| {
            if line.contains(syntax.open) || line.contains(syntax.close) {
                format!("{line} # omnirepo-escaped")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build the exact replacement content for an existing section.  The
/// outside lines (before the open marker and after the close marker) are
/// preserved verbatim.
pub fn build_section_replacement(
    original: &str,
    syntax: &DelimiterSyntax,
    bounds: Bounds,
    payload: &str,
) -> Result<SectionReplacement, SectionError> {
    let lines: Vec<&str> = original.lines().collect();
    let (start, end) = (bounds.start_line as usize, bounds.end_line as usize);
    if start < 1 || end > lines.len() || start >= end {
        return Err(SectionError::BoundsOutside {
            reason: format!("bounds {start}..={end} do not fit the content"),
        });
    }
    let escaped = escape_payload(payload, syntax);
    let mut content = String::new();
    for (index, line) in lines.iter().enumerate() {
        let line_number = index + 1;
        if line_number == start {
            content.push_str(line);
            content.push('\n');
            // The section body: marker lines only when the payload is
            // non-empty.
            if !escaped.is_empty() {
                for body_line in escaped.lines() {
                    content.push_str(body_line);
                    content.push('\n');
                }
            }
        } else if line_number == end {
            content.push_str(line);
            content.push('\n');
        } else if line_number > start && line_number < end {
            // The old body is replaced by the payload.
            continue;
        } else {
            content.push_str(line);
            content.push('\n');
        }
    }
    // An original without a trailing newline stays without one.
    if !original.ends_with('\n') && content.ends_with('\n') {
        content.pop();
    }
    let changed = section_body(original, syntax, bounds) != escaped;
    Ok(SectionReplacement { content, changed })
}

/// The current section body (marker lines excluded).
fn section_body(original: &str, syntax: &DelimiterSyntax, bounds: Bounds) -> String {
    let lines: Vec<&str> = original.lines().collect();
    lines
        .iter()
        .skip(bounds.start_line as usize)
        .take(bounds.end_line as usize - bounds.start_line as usize - 1)
        .filter(|line| !line.contains(syntax.open) && !line.contains(syntax.close))
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod section_builder_tests;
