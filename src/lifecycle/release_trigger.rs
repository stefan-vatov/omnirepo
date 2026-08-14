//! The owner-selected exact-SHA release trigger.
//!
//! A release is triggered only by a version tag whose annotated commit
//! equals the workflow's exact-SHA input, and the repository HEAD must
//! equal that exact SHA.  A tag-commit mismatch or a head mismatch
//! refuses the trigger — no release proceeds from an inexact SHA.

#![allow(dead_code)]

#[cfg(test)]
mod release_trigger_tests;

use std::{error::Error, fmt, path::Path, process::Command};

/// The trigger verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerVerification {
    pub verified: bool,
    pub commit: String,
}

/// Trigger failures.
#[derive(Debug)]
pub enum TriggerError {
    TagMismatch {
        tag: String,
        expected: String,
        actual: String,
    },
    HeadMismatch {
        expected: String,
        actual: String,
    },
}

impl fmt::Display for TriggerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TagMismatch {
                tag,
                expected,
                actual,
            } => write!(
                formatter,
                "the tag {tag:?} points at {actual} but the exact-SHA input is {expected}"
            ),
            Self::HeadMismatch { expected, actual } => write!(
                formatter,
                "the repository HEAD {actual} does not match the exact-SHA input {expected}"
            ),
        }
    }
}
impl Error for TriggerError {}

/// Verify the exact-SHA trigger: the tag's commit and the repository
/// HEAD must both equal the exact-SHA input.
pub fn verify_exact_sha_trigger(
    repo: &Path,
    tag: &str,
    tag_commit: &str,
    head: &str,
) -> Result<TriggerVerification, TriggerError> {
    let actual_tag = git_text(
        repo,
        &["rev-parse", "--verify", &format!("{tag}^{{commit}}")],
    );
    if actual_tag != tag_commit {
        return Err(TriggerError::TagMismatch {
            tag: tag.to_owned(),
            expected: tag_commit.to_owned(),
            actual: actual_tag,
        });
    }
    let actual_head = git_text(repo, &["rev-parse", "HEAD"]);
    if actual_head != head {
        return Err(TriggerError::HeadMismatch {
            expected: head.to_owned(),
            actual: actual_head,
        });
    }
    Ok(TriggerVerification {
        verified: true,
        commit: actual_head,
    })
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
