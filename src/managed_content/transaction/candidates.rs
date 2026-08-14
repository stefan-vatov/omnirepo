use super::*;
use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

pub(crate) fn validate_relative_path(
    path: &Path,
    field: &'static str,
) -> Result<Vec<String>, PlanError> {
    let raw = path.to_str().ok_or(PlanError::InvalidUtf8 { field })?;
    if raw.is_empty() {
        return Err(PlanError::EmptyPath { field });
    }
    if raw.starts_with('/') {
        return Err(PlanError::AbsolutePath {
            field,
            path: raw.to_owned(),
        });
    }
    if raw.contains('\\') {
        return Err(PlanError::InvalidSeparator {
            field,
            path: raw.to_owned(),
        });
    }
    let mut components = Vec::new();
    for (index, component) in raw.split('/').enumerate() {
        if component.is_empty() {
            return Err(PlanError::EmptyComponent {
                field,
                path: raw.to_owned(),
            });
        }
        if component == "." {
            return Err(PlanError::CurrentDirectoryComponent {
                field,
                path: raw.to_owned(),
            });
        }
        if component == ".." {
            return Err(PlanError::ParentTraversal {
                field,
                path: raw.to_owned(),
            });
        }
        if index == 0 && component.ends_with(':') {
            return Err(PlanError::WindowsPrefix {
                field,
                path: raw.to_owned(),
            });
        }
        components.push(component.to_owned());
    }
    Ok(components)
}

/// A candidate temporary path and its strictly increasing collision attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempCandidate {
    path: PathBuf,
    attempt: u32,
}

impl TempCandidate {
    pub fn new(path: PathBuf, attempt: u32) -> Result<Self, CandidateError> {
        if path.as_os_str().is_empty() {
            return Err(CandidateError::EmptyPath);
        }
        validate_relative_path(&path, "temporary candidate")
            .map_err(CandidateError::InvalidPath)?;
        if attempt == 0 {
            return Err(CandidateError::ZeroAttempt);
        }
        Ok(Self { path, attempt })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateError {
    EmptyPath,
    ZeroAttempt,
    EmptyOwnerToken,
    InvalidPath(PlanError),
}

impl fmt::Display for CandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("temporary candidate path is empty"),
            Self::ZeroAttempt => {
                formatter.write_str("temporary candidate attempt must be positive")
            }
            Self::EmptyOwnerToken => formatter.write_str("temporary owner token is empty"),
            Self::InvalidPath(error) => error.fmt(formatter),
        }
    }
}

impl Error for CandidateError {}
