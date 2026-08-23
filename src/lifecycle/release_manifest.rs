//! The release-candidate manifest and exact identity.
//!
//! A non-public release candidate is identified by its exact identity:
//! the version, the exact source commit, the toolchain, and the
//! manifest's own content hash.  The manifest lists the candidate
//! artifacts (name + sha256) and the verified gates.  The exact
//! identity is deterministic: identical inputs yield an identical
//! identity, and any input change changes it.

#![allow(dead_code)]

#[cfg(test)]
mod release_manifest_tests;

use crate::lifecycle::release_build::sha256_hex;
use std::{error::Error, fmt};

/// One candidate artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRef {
    pub name: String,
    pub sha256: String,
}

/// One verified gate result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
}

/// The exact release identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseIdentity {
    pub version: String,
    pub source_commit: String,
    pub toolchain: String,
    pub manifest_sha256: String,
}

/// The candidate manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateManifest {
    pub schema: String,
    pub identity: ReleaseIdentity,
    pub artifacts: Vec<ArtifactRef>,
    pub gates: Vec<GateResult>,
}

/// Manifest failures.
#[derive(Debug)]
pub enum ManifestError {
    InvalidVersion { version: String },
    InvalidCommit { commit: String },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion { version } => {
                write!(formatter, "invalid release version {version:?}")
            }
            Self::InvalidCommit { commit } => {
                write!(formatter, "invalid source commit {commit:?}")
            }
        }
    }
}
impl Error for ManifestError {}

pub(super) fn valid_version(version: &str) -> bool {
    semver::Version::parse(version).is_ok()
}

fn valid_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Build the candidate manifest for the exact inputs.
pub fn manifest_for(
    version: &str,
    source_commit: &str,
    toolchain: &str,
    artifacts: Vec<ArtifactRef>,
    gates: Vec<GateResult>,
) -> Result<CandidateManifest, ManifestError> {
    if !valid_version(version) {
        return Err(ManifestError::InvalidVersion {
            version: version.to_owned(),
        });
    }
    if !valid_commit(source_commit) {
        return Err(ManifestError::InvalidCommit {
            commit: source_commit.to_owned(),
        });
    }
    let mut manifest = CandidateManifest {
        schema: "omnirepo.release-candidate.v1".to_owned(),
        identity: ReleaseIdentity {
            version: version.to_owned(),
            source_commit: source_commit.to_owned(),
            toolchain: toolchain.to_owned(),
            manifest_sha256: String::new(),
        },
        artifacts,
        gates,
    };
    manifest.identity.manifest_sha256 = content_hash(&manifest);
    Ok(manifest)
}

/// The canonical exact identity string: the content hash over the
/// identity and manifest fields, deterministic.
pub fn exact_identity(manifest: &CandidateManifest) -> String {
    format!(
        "omnirepo-{}@{}[{}|{}|{}|{}]",
        manifest.identity.version,
        manifest.identity.source_commit,
        manifest.identity.toolchain,
        manifest.identity.manifest_sha256,
        manifest.artifacts.len(),
        manifest.gates.len(),
    )
}

pub(crate) fn content_hash(manifest: &CandidateManifest) -> String {
    let mut content = Vec::new();
    let mut absorb = |bytes: &[u8]| {
        content.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        content.extend_from_slice(bytes);
    };
    absorb(manifest.schema.as_bytes());
    absorb(manifest.identity.version.as_bytes());
    absorb(manifest.identity.source_commit.as_bytes());
    absorb(manifest.identity.toolchain.as_bytes());
    absorb(&(manifest.artifacts.len() as u64).to_be_bytes());
    for artifact in &manifest.artifacts {
        absorb(artifact.name.as_bytes());
        absorb(artifact.sha256.as_bytes());
    }
    absorb(&(manifest.gates.len() as u64).to_be_bytes());
    for gate in &manifest.gates {
        absorb(gate.name.as_bytes());
        absorb(&[u8::from(gate.passed)]);
    }
    sha256_hex(&content)
}
