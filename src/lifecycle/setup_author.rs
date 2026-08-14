//! Idempotent machine authority authoring.
//!
//! Applies the setup effect plan to the explicitly selected canonical
//! machine configuration: create when absent, no-op when identical,
//! update when valid but different — never replace an invalid or
//! conflicting authority.  The write is atomic (temp + rename) and
//! repeated application leaves the authority in the same state.

#![allow(dead_code)]

use crate::lifecycle::setup_plan::{
    ExistingFile, SetupAction, SetupIntent, SetupPlanError, compute_setup_plan,
};

#[cfg(test)]
mod setup_author_tests;
use std::path::Path;

/// Observe the existing canonical file state at the selected path.
pub fn observe_existing(root: &Path, path: &str) -> Vec<ExistingFile> {
    let full = root.join(path);
    match std::fs::read_to_string(&full) {
        Ok(content) => vec![ExistingFile {
            path: path.to_owned(),
            content: content.clone(),
            valid: crate::configuration::parse_yaml_subset(&content).is_ok(),
        }],
        Err(_) => Vec::new(),
    }
}

/// Apply the setup plan for the selected machine configuration.
///
/// The write is atomic: the content is written to a temporary name and
/// renamed into place.  An invalid or conflicting authority is refused
/// and stays byte-identical.
pub fn apply_setup_plan(
    root: &Path,
    intent: &SetupIntent,
    existing: &[ExistingFile],
) -> Result<SetupAction, SetupPlanError> {
    let plan = compute_setup_plan(intent, existing)?;
    let action = plan.into_iter().next().expect("one selected file");
    match &action {
        SetupAction::Create { path } | SetupAction::Update { path } => {
            let target = root.join(path);
            let temporary = root.join(format!(".{}.tmp-{}", path, std::process::id()));
            std::fs::write(&temporary, &intent.machine_content).map_err(|error| {
                SetupPlanError::Io {
                    path: temporary.display().to_string(),
                    reason: error.to_string(),
                }
            })?;
            std::fs::rename(&temporary, &target).map_err(|error| SetupPlanError::Io {
                path: target.display().to_string(),
                reason: error.to_string(),
            })?;
        }
        SetupAction::NoOp { .. } => {}
    }
    Ok(action)
}
