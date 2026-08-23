//! Re-run the normative gates and verify candidate provenance.
//!
//! The gate orchestrator runs every configured gate over the exact-SHA
//! candidate and collects each result (a failing gate never stops the
//! others).  Provenance verification ties the candidate manifest to the
//! checkout: the manifest's source commit must equal the checkout HEAD
//! and the manifest's content hash must match its own identity — a
//! tampered manifest is refused.

#![allow(dead_code)]

use crate::lifecycle::release_manifest::{CandidateManifest, content_hash};

#[cfg(test)]
mod release_gates_tests;
use std::{error::Error, fmt, path::Path, process::Command};

/// One gate result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateRun {
    pub name: String,
    pub passed: bool,
    pub evidence: String,
}

/// Provenance failures.
#[derive(Debug)]
pub enum ProvenanceError {
    HeadUnavailable {
        path: std::path::PathBuf,
        reason: String,
    },
    CommitMismatch {
        expected: String,
        actual: String,
    },
    ManifestTampered {
        expected: String,
        actual: String,
    },
}

impl fmt::Display for ProvenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeadUnavailable { path, reason } => {
                write!(
                    formatter,
                    "checkout head unavailable {}: {reason}",
                    path.display()
                )
            }
            Self::CommitMismatch { expected, actual } => write!(
                formatter,
                "the checkout HEAD {actual} does not match the manifest commit {expected}"
            ),
            Self::ManifestTampered { expected, actual } => write!(
                formatter,
                "the manifest content hash {actual} does not match its identity {expected}"
            ),
        }
    }
}
impl Error for ProvenanceError {}

/// Run every configured gate over the candidate.  A failing gate is
/// collected, never stopping the others.  Gates are explicit argument
/// arrays — never shell strings.
pub fn run_normative_gates(gates: &[(String, Vec<String>)]) -> Vec<GateRun> {
    gates
        .iter()
        .map(|(name, argv)| {
            // The bounded ETXTBSY retry (the shared check-runner pattern)
            // makes spawning a just-materialized gate script robust.
            let mut command = Command::new(&argv[0]);
            command
                .args(&argv[1..])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let child = match crate::lifecycle::check_runner::spawn_retry(&mut command) {
                Ok(child) => child,
                Err(error) => {
                    return GateRun {
                        name: name.clone(),
                        passed: false,
                        evidence: format!("cannot start gate: {error}"),
                    };
                }
            };
            let output = match child.wait_with_output() {
                Ok(output) => output,
                Err(error) => {
                    return GateRun {
                        name: name.clone(),
                        passed: false,
                        evidence: format!("cannot collect gate output: {error}"),
                    };
                }
            };
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            GateRun {
                name: name.clone(),
                passed: output.status.success(),
                evidence: format!("{stdout}{stderr}"),
            }
        })
        .collect()
}

/// Verify the candidate provenance against the checkout: the manifest's
/// source commit equals the checkout HEAD, and the manifest's content
/// hash matches its own identity (no tampering).
pub fn verify_candidate_provenance(
    manifest: &CandidateManifest,
    checkout: &Path,
) -> Result<(), ProvenanceError> {
    let head_file = checkout.join("HEAD");
    let head =
        std::fs::read_to_string(&head_file).map_err(|error| ProvenanceError::HeadUnavailable {
            path: head_file.clone(),
            reason: error.to_string(),
        })?;
    let head = head.trim().to_owned();
    if head != manifest.identity.source_commit {
        return Err(ProvenanceError::CommitMismatch {
            expected: manifest.identity.source_commit.clone(),
            actual: head,
        });
    }
    let expected = content_hash(manifest);
    if expected != manifest.identity.manifest_sha256 {
        return Err(ProvenanceError::ManifestTampered {
            expected,
            actual: manifest.identity.manifest_sha256.clone(),
        });
    }
    Ok(())
}
