//! Connect setup completion to the first inferred synchronization.
//!
//! Setup computes and displays the effect plan over the explicitly
//! selected canonical files; apply requires the explicit confirmation
//! contract (interactive confirmation or `--yes` in non-interactive
//! use) and authors the machine configuration idempotently; after apply,
//! the next synchronization runs the inferred first pass over the
//! authored authority.  An invalid or conflicting authority is never
//! replaced.

#![allow(dead_code)]

#[cfg(test)]
mod setup_run_tests;

use crate::lifecycle::setup_author::{apply_setup_plan, observe_existing};
use crate::lifecycle::setup_plan::{SetupAction, SetupIntent, SetupPlanError};
use std::path::Path;

/// The setup request for the canonical machine configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub path: String,
    pub content: String,
    pub apply: bool,
    pub confirmed: bool,
}

impl SetupRequest {
    pub fn machine(path: String, content: String, apply: bool, confirmed: bool) -> Self {
        Self {
            path,
            content,
            apply,
            confirmed,
        }
    }
}

/// The setup outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupOutcome {
    /// The displayed effect plan (the actions setup would take).
    pub plan: Vec<SetupAction>,
    /// The actions actually applied to the filesystem.
    pub applied: Vec<SetupAction>,
}

/// Setup failures.
#[derive(Debug)]
pub enum SetupRunError {
    ConfirmationRequired,
    ConflictingAuthority { path: String },
    Io { path: String, reason: String },
    Plan(SetupPlanError),
}

impl std::fmt::Display for SetupRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationRequired => {
                write!(
                    formatter,
                    "setup apply requires an explicit confirmation (interactive or --yes)"
                )
            }
            Self::ConflictingAuthority { path } => {
                write!(
                    formatter,
                    "the existing authority {path:?} is never replaced"
                )
            }
            Self::Io { path, reason } => {
                write!(formatter, "setup io failure {path:?}: {reason}")
            }
            Self::Plan(error) => write!(formatter, "setup plan failure: {error}"),
        }
    }
}
impl std::error::Error for SetupRunError {}

/// Run setup for the selected machine configuration.
///
/// Without apply, the plan is computed and returned (display); with
/// apply, the explicit confirmation contract gates the write.  After a
/// successful apply the authored authority is ready for the first
/// inferred synchronization (the sync command runs the fleet dispatch
/// over it).
pub fn run_setup(
    home: &Path,
    request: &SetupRequest,
    _first_sync: Option<()>,
) -> Result<SetupOutcome, SetupRunError> {
    let existing = observe_existing(home, &request.path);
    let intent = SetupIntent::machine(&request.path, &request.content);
    if request.apply {
        if !request.confirmed {
            return Err(SetupRunError::ConfirmationRequired);
        }
        let action = apply_setup_plan(home, &intent, &existing).map_err(|error| match error {
            SetupPlanError::ConflictingAuthority { path } => {
                SetupRunError::ConflictingAuthority { path }
            }
            SetupPlanError::Io { path, reason } => SetupRunError::Io { path, reason },
        })?;
        let applied = match &action {
            SetupAction::NoOp { .. } => Vec::new(),
            _ => vec![action.clone()],
        };
        Ok(SetupOutcome {
            plan: vec![action],
            applied,
        })
    } else {
        let plan = crate::lifecycle::setup_plan::compute_setup_plan(&intent, &existing)
            .map_err(SetupRunError::Plan)?;
        Ok(SetupOutcome {
            applied: Vec::new(),
            plan,
        })
    }
}
