//! Named-section scanning and delimiter topology classification.
//!
//! A file may carry any number of named, non-overlapping managed
//! sections.  Scanning classifies every line against the format's exact
//! named markers and yields the ordered sections, or one contextual
//! failure for any ambiguous topology: unpaired, reversed, nested,
//! interleaved, duplicate-ID, mismatched-ID, or payload-like marker
//! lines (canon/architecture/managed-content.md).  Scanning is pure and
//! byte-exact: the original content is never touched or decoded.

#![allow(dead_code)]

use super::delimiters::{DelimiterSyntax, LineClass};
use crate::configuration::SectionId;
use std::{error::Error, fmt};

/// The bounds of one managed section: 1-based marker line numbers,
/// inclusive of both marker lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub start_line: u64,
    pub end_line: u64,
}

/// One named section found in a file, in file order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedSection {
    pub id: SectionId,
    pub bounds: Bounds,
}

/// The scanned topology: every named section, or one contextual failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanOutcome {
    /// Every marker line resolved into ordered, non-nested named pairs
    /// (the list is empty when no markers are present).
    Sections(Vec<NamedSection>),
    /// The topology is ambiguous; the reason names the failure.
    Invalid { reason: String },
}

impl fmt::Display for ScanOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sections(sections) if sections.is_empty() => {
                write!(formatter, "no managed section markers present")
            }
            Self::Sections(sections) => {
                write!(formatter, "{} managed section(s)", sections.len())
            }
            Self::Invalid { reason } => {
                write!(formatter, "ambiguous delimiter topology: {reason}")
            }
        }
    }
}
impl Error for ScanOutcome {}

/// Split content into lines that keep their exact terminators.
pub fn split_inclusive_lines(content: &[u8]) -> Vec<&[u8]> {
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
    lines
}

/// The logical line: the chunk without its LF or CRLF terminator.
pub fn logical_line(chunk: &[u8]) -> &[u8] {
    let chunk = chunk.strip_suffix(b"\n").unwrap_or(chunk);
    chunk.strip_suffix(b"\r").unwrap_or(chunk)
}

/// Scan the content for named marker pairs and classify the topology.
/// Pure: no mutation, no I/O, no decoding of payload bytes.
pub fn scan_sections(content: &[u8], syntax: &DelimiterSyntax) -> ScanOutcome {
    let mut sections: Vec<NamedSection> = Vec::new();
    let mut open: Option<(SectionId, u64)> = None;
    for (index, chunk) in split_inclusive_lines(content).iter().enumerate() {
        let line_number = (index + 1) as u64;
        match syntax.classify_line(logical_line(chunk)) {
            LineClass::Content => {}
            LineClass::MarkerLike { reason } => {
                return ScanOutcome::Invalid {
                    reason: format!(
                        "line {line_number} resembles a marker but is invalid: {reason}"
                    ),
                };
            }
            LineClass::Open(id) => {
                if let Some((outer, _)) = &open {
                    return ScanOutcome::Invalid {
                        reason: format!(
                            "line {line_number} opens section {id} inside the open section {outer}"
                        ),
                    };
                }
                if sections.iter().any(|section| section.id == id) {
                    return ScanOutcome::Invalid {
                        reason: format!("line {line_number} opens the duplicate section id {id}"),
                    };
                }
                open = Some((id, line_number));
            }
            LineClass::Close(id) => match open.take() {
                None => {
                    return ScanOutcome::Invalid {
                        reason: format!(
                            "line {line_number} closes section {id} without an open marker"
                        ),
                    };
                }
                Some((opened, start_line)) if opened == id => {
                    sections.push(NamedSection {
                        id,
                        bounds: Bounds {
                            start_line,
                            end_line: line_number,
                        },
                    });
                }
                Some((opened, _)) => {
                    return ScanOutcome::Invalid {
                        reason: format!(
                            "line {line_number} closes section {id} while section {opened} is open"
                        ),
                    };
                }
            },
        }
    }
    if let Some((id, start_line)) = open {
        return ScanOutcome::Invalid {
            reason: format!("the open marker for section {id} at line {start_line} is unclosed"),
        };
    }
    ScanOutcome::Sections(sections)
}

#[cfg(test)]
mod partial_scan_tests;
