use super::*;
use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
/// Parent directories that the current operation may remove on failure.
pub enum ParentDirectories {
    Existing,
    Created(Vec<PathBuf>),
}

impl ParentDirectories {
    pub fn existing() -> Self {
        Self::Existing
    }

    pub fn created<I, S>(parents: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<PathBuf>,
    {
        Self::Created(parents.into_iter().map(Into::into).collect())
    }

    pub(crate) fn requires_cleanup(&self) -> bool {
        matches!(self, Self::Created(parents) if !parents.is_empty())
    }
}

/// The plan identity used to make temporary names operation-local.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionPlan {
    pub(crate) operation_id: String,
    target: PathBuf,
    pub(crate) parents: ParentDirectories,
}

impl TransactionPlan {
    pub fn new(
        operation_id: impl Into<String>,
        target: PathBuf,
        parents: ParentDirectories,
    ) -> Result<Self, PlanError> {
        let operation_id = operation_id.into();
        if operation_id.is_empty() {
            return Err(PlanError::EmptyOperationId);
        }
        let target_components = validate_relative_path(&target, "target")?;
        let target_parent_len = target_components.len().saturating_sub(1);
        if let ParentDirectories::Created(created) = &parents {
            let mut validated = Vec::with_capacity(created.len());
            for parent in created {
                let components = validate_relative_path(parent, "created parent")?;
                if components.len() > target_parent_len
                    || components != target_components[..components.len()]
                {
                    return Err(PlanError::ParentOutsideTarget {
                        path: parent.display().to_string(),
                    });
                }
                if validated
                    .iter()
                    .any(|existing: &Vec<String>| existing == &components)
                {
                    return Err(PlanError::DuplicateParent {
                        path: parent.display().to_string(),
                    });
                }
                validated.push(components);
            }
        }
        Ok(Self {
            operation_id,
            target,
            parents,
        })
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn parents(&self) -> &ParentDirectories {
        &self.parents
    }

    pub(crate) fn target_parent(&self) -> Option<&Path> {
        self.target.parent()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    EmptyOperationId,
    EmptyPath { field: &'static str },
    InvalidUtf8 { field: &'static str },
    AbsolutePath { field: &'static str, path: String },
    InvalidSeparator { field: &'static str, path: String },
    EmptyComponent { field: &'static str, path: String },
    CurrentDirectoryComponent { field: &'static str, path: String },
    ParentTraversal { field: &'static str, path: String },
    WindowsPrefix { field: &'static str, path: String },
    ParentOutsideTarget { path: String },
    DuplicateParent { path: String },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyOperationId => formatter.write_str("transaction operation id is empty"),
            Self::EmptyPath { field } => write!(formatter, "{field} path is empty"),
            Self::InvalidUtf8 { field } => write!(formatter, "{field} path is not UTF-8"),
            Self::AbsolutePath { field, path } => {
                write!(formatter, "{field} path is absolute: {path:?}")
            }
            Self::InvalidSeparator { field, path } => {
                write!(
                    formatter,
                    "{field} path uses an unsupported separator: {path:?}"
                )
            }
            Self::EmptyComponent { field, path } => {
                write!(formatter, "{field} path has an empty component: {path:?}")
            }
            Self::CurrentDirectoryComponent { field, path } => {
                write!(
                    formatter,
                    "{field} path has a current-directory component: {path:?}"
                )
            }
            Self::ParentTraversal { field, path } => {
                write!(formatter, "{field} path traverses a parent: {path:?}")
            }
            Self::WindowsPrefix { field, path } => {
                write!(formatter, "{field} path has a drive prefix: {path:?}")
            }
            Self::ParentOutsideTarget { path } => {
                write!(
                    formatter,
                    "created parent is outside the target path: {path:?}"
                )
            }
            Self::DuplicateParent { path } => {
                write!(formatter, "created parent is duplicated: {path:?}")
            }
        }
    }
}

impl Error for PlanError {}
