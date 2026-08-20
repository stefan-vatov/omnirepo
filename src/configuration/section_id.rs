//! Section identity: the one owner of the managed-section ID rule.
//!
//! IDs are explicit, stable, exact, and case-sensitive: ASCII lowercase
//! letters, digits, dots, underscores, and hyphens
//! (canon/architecture/managed-content.md).  Every layer — declarations,
//! resolution, markers, Git ownership — validates through this rule.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// One validated managed-section ID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SectionId(String);

/// Section-ID failures.
#[derive(Debug)]
pub struct SectionIdError {
    pub value: String,
}

impl fmt::Display for SectionIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid section id {:?}: ids use ASCII lowercase letters, digits, dots, underscores, and hyphens",
            self.value
        )
    }
}
impl Error for SectionIdError {}

/// The exact character rule for section IDs.
pub fn is_valid_section_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

impl SectionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SectionIdError> {
        let value = value.into();
        if !is_valid_section_id(&value) {
            return Err(SectionIdError { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
