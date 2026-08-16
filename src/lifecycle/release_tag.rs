//! Create or validate the canonical release tag and identity.
//!
//! The canonical tag is `v<version>` — an annotated tag at the exact
//! source commit.  Creation is idempotent: an existing tag at the same
//! commit is reported as existing; an existing tag at a different
//! commit is refused (the release identity would be ambiguous).
//! Validation checks the name is canonical, the tag exists, is
//! annotated, and points at the exact commit.

#![allow(dead_code)]

#[cfg(test)]
mod release_tag_tests;

use std::{error::Error, fmt, path::Path, process::Command};

/// The tag creation outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TagOutcome {
    Created { oid: String },
    Existing { oid: String },
    Refused { reason: String },
}

/// The tag validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagValidation {
    pub annotated: bool,
    pub commit: String,
}

/// Tag failures.
#[derive(Debug)]
pub enum TagError {
    Missing {
        tag: String,
    },
    NonCanonicalName {
        tag: String,
    },
    NotAnnotated {
        tag: String,
    },
    Io {
        path: std::path::PathBuf,
        reason: String,
    },
}

impl fmt::Display for TagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { tag } => write!(formatter, "the release tag {tag:?} does not exist"),
            Self::NonCanonicalName { tag } => {
                write!(
                    formatter,
                    "the tag name {tag:?} is not canonical (expected v<version>)"
                )
            }
            Self::NotAnnotated { tag } => {
                write!(formatter, "the release tag {tag:?} is not annotated")
            }
            Self::Io { path, reason } => {
                write!(formatter, "tag io failure {}: {reason}", path.display())
            }
        }
    }
}
impl Error for TagError {}

/// Create the canonical annotated tag at the exact commit.
pub fn create_canonical_tag(
    repo: &str,
    version: &str,
    commit: &str,
) -> Result<TagOutcome, TagError> {
    let tag = format!("v{version}");
    if !is_canonical_name(&tag) {
        return Err(TagError::NonCanonicalName { tag });
    }
    let existing = git_text(
        Path::new(repo),
        &["rev-parse", "--verify", &format!("{tag}^{{commit}}")],
    );
    if !existing.is_empty() {
        if existing == commit {
            return Ok(TagOutcome::Existing { oid: existing });
        }
        return Ok(TagOutcome::Refused {
            reason: format!("the tag {tag:?} already exists at {existing}, not at {commit}"),
        });
    }
    let output = Command::new("git")
        .args([
            "tag",
            "-a",
            &tag,
            "-m",
            &format!("omnirepo {version}"),
            commit,
        ])
        .current_dir(repo)
        .output()
        .map_err(|error| TagError::Io {
            path: Path::new(repo).to_path_buf(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(TagError::Io {
            path: Path::new(repo).to_path_buf(),
            reason: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(TagOutcome::Created {
        oid: commit.to_owned(),
    })
}

/// Validate the canonical release tag.
pub fn validate_canonical_tag(repo: &str, version: &str) -> Result<TagValidation, TagError> {
    let tag = format!("v{version}");
    if !is_canonical_name(&tag) {
        return Err(TagError::NonCanonicalName { tag });
    }
    let object = git_text(
        Path::new(repo),
        &["rev-parse", "--verify", &format!("{tag}^{{object}}")],
    );
    if object.is_empty() {
        return Err(TagError::Missing { tag });
    }
    // Annotated tags resolve to a tag object, not directly to a commit.
    let kind = git_text(Path::new(repo), &["cat-file", "-t", &object]);
    let annotated = kind == "tag";
    let commit = git_text(
        Path::new(repo),
        &["rev-parse", "--verify", &format!("{tag}^{{commit}}")],
    );
    if commit.is_empty() {
        return Err(TagError::Missing { tag });
    }
    if !annotated {
        return Err(TagError::NotAnnotated { tag });
    }
    Ok(TagValidation { annotated, commit })
}

fn is_canonical_name(tag: &str) -> bool {
    tag.starts_with('v')
        && tag[1..].split('.').count() >= 2
        && tag[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
