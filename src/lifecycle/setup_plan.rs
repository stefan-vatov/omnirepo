//! Model setup intent and existing-state effect plans.
//!
//! Setup first computes and displays an effect plan over the explicitly
//! selected canonical configuration files only — never an ambient fleet.
//! An absent file plans a create; an identical valid file plans a
//! no-op (repeated setup is a no-op); a valid but different file plans
//! an update; an invalid or conflicting file is refused — never
//! replaced.

#![allow(dead_code)]

#[cfg(test)]
mod setup_plan_tests;

use std::{error::Error, fmt};

/// The observed state of one canonical configuration file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExistingFile {
    pub path: String,
    pub content: String,
    pub valid: bool,
}

/// The setup intent: one explicitly selected machine-configuration
/// file (the canonical `<HOME>/.omnirepo/config.yaml` content).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupIntent {
    pub machine_path: String,
    pub machine_content: String,
}

impl SetupIntent {
    pub fn machine(path: &str, content: &str) -> Self {
        Self {
            machine_path: path.to_owned(),
            machine_content: content.to_owned(),
        }
    }
}

/// One planned setup effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupAction {
    Create { path: String },
    Update { path: String },
    NoOp { path: String, reason: String },
}

/// Setup planning failures.
#[derive(Debug)]
pub enum SetupPlanError {
    ConflictingAuthority { path: String },
    Io { path: String, reason: String },
}

impl fmt::Display for SetupPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingAuthority { path } => {
                write!(
                    formatter,
                    "the existing authority file {path:?} is invalid or conflicting and is never replaced"
                )
            }
            Self::Io { path, reason } => {
                write!(formatter, "setup authority io failure {path:?}: {reason}")
            }
        }
    }
}
impl Error for SetupPlanError {}

/// Compute the setup effect plan over the explicitly selected files.
///
/// The plan considers only the selected canonical file; unrelated files
/// never enter it (no ambient fleet discovery).
pub fn compute_setup_plan(
    intent: &SetupIntent,
    existing: &[ExistingFile],
) -> Result<Vec<SetupAction>, SetupPlanError> {
    let current = existing
        .iter()
        .find(|file| file.path == intent.machine_path);
    let action = match current {
        None => SetupAction::Create {
            path: intent.machine_path.clone(),
        },
        Some(file) if !file.valid => {
            return Err(SetupPlanError::ConflictingAuthority {
                path: intent.machine_path.clone(),
            });
        }
        Some(file) if file.content == intent.machine_content => SetupAction::NoOp {
            path: intent.machine_path.clone(),
            reason: "the canonical machine configuration already matches the intent".to_owned(),
        },
        Some(_) => SetupAction::Update {
            path: intent.machine_path.clone(),
        },
    };
    Ok(vec![action])
}
