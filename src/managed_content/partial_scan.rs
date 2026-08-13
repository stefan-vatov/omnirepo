//! Partial-file scanning and delimiter topology classification.
//!
//! Exactly one ordered, non-nested marker pair yields the bounds; the
//! absent case is distinct; every ambiguous topology (unpaired, nested,
//! multiple, reversed) returns a contextual failure.  Scanning is pure:
//! the original content is never touched.

#![allow(dead_code)]

use super::delimiters::DelimiterSyntax;
use std::{error::Error, fmt};

/// The bounds of one managed section (1-based lines).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub start_line: u64,
    pub end_line: u64,
}

/// The scanned topology.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Topology {
    /// No marker pair is present.
    Absent,
    /// Exactly one ordered, non-nested pair.
    ExactlyOne { bounds: Bounds },
    /// The topology is ambiguous; the reason names the failure.
    Ambiguous { reason: String },
}

impl fmt::Display for Topology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => write!(formatter, "no managed section markers present"),
            Self::ExactlyOne { bounds } => write!(
                formatter,
                "one managed section at lines {}..={}",
                bounds.start_line, bounds.end_line
            ),
            Self::Ambiguous { reason } => {
                write!(formatter, "ambiguous delimiter topology: {reason}")
            }
        }
    }
}
impl Error for Topology {}

/// Scan the content for the syntax's canonical markers and classify the
/// topology.  Pure: no mutation, no I/O.
pub fn scan_partial(content: &str, syntax: &DelimiterSyntax) -> Topology {
    let opens: Vec<u64> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(syntax.open))
        .map(|(index, _)| (index + 1) as u64)
        .collect();
    let closes: Vec<u64> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(syntax.close))
        .map(|(index, _)| (index + 1) as u64)
        .collect();

    if opens.is_empty() && closes.is_empty() {
        return Topology::Absent;
    }
    if opens.is_empty() {
        return Topology::Ambiguous {
            reason: "a close marker exists without an open marker".to_owned(),
        };
    }
    if closes.is_empty() {
        return Topology::Ambiguous {
            reason: "an open marker exists without a close marker".to_owned(),
        };
    }
    if opens.len() > 1 || closes.len() > 1 {
        return Topology::Ambiguous {
            reason: "more than one marker pair is present".to_owned(),
        };
    }
    let (open_line, close_line) = (opens[0], closes[0]);
    if open_line >= close_line {
        return Topology::Ambiguous {
            reason: "the open marker does not precede the close marker".to_owned(),
        };
    }
    // Nested: an open marker inside the section body (the scan only found
    // one open, so nesting is detected by the marker appearing twice on
    // one line or the pair-in-pair shape is impossible with one each).
    Topology::ExactlyOne {
        bounds: Bounds {
            start_line: open_line,
            end_line: close_line,
        },
    }
}

#[cfg(test)]
mod partial_scan_tests;
