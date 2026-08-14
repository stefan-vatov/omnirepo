//! Source declaration and destination policy authoring.
//!
//! The generic canonical-file author: observe the existing state,
//! compute the setup effect plan (create / no-op / update / refuse) over
//! the explicitly selected file, and apply it atomically.  The validity
//! check is specific to each file kind — the declarations parser for
//! `<source-root>/.omnirepo/source.yaml` and the YAML subset for
//! `.omnirepo.yaml` — so an invalid or conflicting authority is never
//! replaced.

#![allow(dead_code)]

#[cfg(test)]
mod setup_files_tests;

use crate::lifecycle::setup_plan::{ExistingFile, SetupAction, SetupPlanError};
use std::path::Path;

/// Is the content valid source declarations?
pub fn is_valid_declarations(content: &str) -> bool {
    use crate::source::{RevisionId, SourceId};
    let source = SourceId::new("source-a");
    let revision = RevisionId::new("rev-1");
    match (source, revision) {
        (Ok(source), Ok(revision)) => crate::source::parse_declarations(
            &source,
            &revision,
            &[("source.yaml", content.to_owned())],
        )
        .is_ok(),
        _ => false,
    }
}

/// Is the content valid subset YAML (the machine and policy files)?
pub fn is_valid_yaml(content: &str) -> bool {
    crate::configuration::parse_yaml_subset(content).is_ok()
}

/// Author one canonical file below the root: observe, plan, and apply
/// with the kind-specific validity check.
pub fn author_canonical_file(
    root: &Path,
    relative: &str,
    content: &str,
    valid: impl Fn(&str) -> bool,
) -> Result<SetupAction, SetupPlanError> {
    let existing = observe(root, relative, valid);
    let action = plan_one(relative, content, &existing)?;
    match &action {
        SetupAction::Create { path } | SetupAction::Update { path } => {
            let target = root.join(path);
            let parent = target.parent().unwrap_or(root);
            let file_name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
            std::fs::write(&temporary, content).map_err(|error| SetupPlanError::Io {
                path: temporary.display().to_string(),
                reason: error.to_string(),
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

/// Observe the existing file state at the selected relative path.
pub fn observe(root: &Path, relative: &str, valid: impl Fn(&str) -> bool) -> Vec<ExistingFile> {
    let full = root.join(relative);
    match std::fs::read_to_string(&full) {
        Ok(content) => vec![ExistingFile {
            path: relative.to_owned(),
            content: content.clone(),
            valid: valid(&content),
        }],
        Err(_) => Vec::new(),
    }
}

/// Compute the single-file plan with the validity-aware refusal.
fn plan_one(
    relative: &str,
    content: &str,
    existing: &[ExistingFile],
) -> Result<SetupAction, SetupPlanError> {
    let current = existing.iter().find(|file| file.path == relative);
    match current {
        None => Ok(SetupAction::Create {
            path: relative.to_owned(),
        }),
        Some(file) if !file.valid => Err(SetupPlanError::ConflictingAuthority {
            path: relative.to_owned(),
        }),
        Some(file) if file.content == content => Ok(SetupAction::NoOp {
            path: relative.to_owned(),
            reason: "the canonical file already matches the intent".to_owned(),
        }),
        Some(_) => Ok(SetupAction::Update {
            path: relative.to_owned(),
        }),
    }
}
