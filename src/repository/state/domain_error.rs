use super::{CausationBasis, CausationRelation, TargetChange};

use std::{error::Error, fmt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    EmptyValue {
        field: &'static str,
    },
    ControlCharacter {
        field: &'static str,
    },
    InvalidAbsoluteRoot {
        value: String,
    },
    InvalidRelativePath {
        value: String,
    },
    InvalidManagedSectionId {
        value: String,
    },
    InvalidRenamePaths {
        from: String,
        to: String,
    },
    AuthorityDeviceMismatch {
        filesystem_device: u64,
        object_device: u64,
    },
    DuplicateValue {
        field: &'static str,
        value: String,
    },
    EmptyEntries {
        field: &'static str,
    },
    InvalidChangeShape {
        change: TargetChange,
    },
    ConflictingTarget {
        path: String,
    },
    UnauthorizedTarget {
        path: String,
    },
    InvalidCausation {
        relation: CausationRelation,
        basis: CausationBasis,
    },
    InvalidProofBinding {
        field: &'static str,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue { field } => write!(formatter, "{field} must not be empty"),
            Self::ControlCharacter { field } => {
                write!(formatter, "{field} must not contain control characters")
            }
            Self::InvalidAbsoluteRoot { value } => {
                write!(formatter, "invalid absolute repository root {value:?}")
            }
            Self::InvalidRelativePath { value } => {
                write!(formatter, "invalid relative repository path {value:?}")
            }
            Self::InvalidManagedSectionId { value } => {
                write!(formatter, "invalid managed section ID {value:?}")
            }
            Self::InvalidRenamePaths { from, to } => {
                write!(
                    formatter,
                    "rename source and destination must differ: {from:?} -> {to:?}"
                )
            }
            Self::AuthorityDeviceMismatch {
                filesystem_device,
                object_device,
            } => write!(
                formatter,
                "authority filesystem/object device mismatch: filesystem={filesystem_device}, object={object_device}"
            ),
            Self::DuplicateValue { field, value } => {
                write!(formatter, "duplicate {field} value {value:?}")
            }
            Self::EmptyEntries { field } => write!(formatter, "{field} entries must not be empty"),
            Self::InvalidChangeShape { change } => {
                write!(formatter, "invalid before/after shape for {change:?}")
            }
            Self::ConflictingTarget { path } => {
                write!(formatter, "conflicting managed target scope at {path:?}")
            }
            Self::UnauthorizedTarget { path } => {
                write!(
                    formatter,
                    "authorized delta target is outside the frozen snapshot: {path:?}"
                )
            }
            Self::InvalidCausation { relation, basis } => {
                write!(
                    formatter,
                    "invalid causation relation {relation:?} with basis {basis:?}"
                )
            }
            Self::InvalidProofBinding { field } => {
                write!(formatter, "causation proof is not bound to {field}")
            }
        }
    }
}

impl Error for DomainError {}

pub(crate) fn validate_text(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    if value.chars().any(char::is_control) {
        return Err(DomainError::ControlCharacter { field });
    }
    Ok(())
}

macro_rules! text_value {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                validate_text(&value, $field)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

text_value!(RepositoryId, "repository ID");
text_value!(RevisionId, "revision ID");
text_value!(RefName, "ref name");
text_value!(CheckWitness, "check witness");
