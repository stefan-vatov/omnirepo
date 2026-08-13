//! Exact source representation preservation.
//!
//! The destination managed representation must equal the source exactly:
//! no normalization and no semantic merge.  An unsupported representation
//! fails typed BEFORE any write; the check is pure.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The representation check outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Representation {
    /// The destination can carry the source bytes exactly.
    Exact,
    /// The source representation is unsupported; nothing is written.
    Unsupported { reason: String },
}

impl fmt::Display for Representation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => write!(formatter, "the representation is exact"),
            Self::Unsupported { reason } => {
                write!(formatter, "unsupported representation: {reason}")
            }
        }
    }
}
impl Error for Representation {}

/// Check that the source bytes can be carried exactly.  No normalization
/// and no semantic merge ever occurs: the check is byte-level only.
pub fn check_exact_representation(
    source_bytes: &[u8],
    target_encoding_utf8: bool,
) -> Representation {
    if !target_encoding_utf8 {
        return Representation::Exact;
    }
    match std::str::from_utf8(source_bytes) {
        Ok(_) => Representation::Exact,
        Err(_) => Representation::Unsupported {
            reason: "the source bytes are not valid UTF-8 for a UTF-8 target".to_owned(),
        },
    }
}

/// The destination representation equals the source bytes exactly when
/// the check passed; this is the no-op comparison used before any write.
pub fn destination_equals_source(destination: &[u8], source: &[u8]) -> bool {
    destination == source
}

#[cfg(test)]
mod representation_tests;
