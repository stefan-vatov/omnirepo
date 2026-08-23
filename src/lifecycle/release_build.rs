//! Build the Cargo package from a clean locked exact-SHA checkout.
//!
//! The packaging gate: the checkout must be clean (no uncommitted or
//! untracked changes) and its HEAD must equal the manifest's exact
//! source commit; then `cargo package --locked` produces the .crate
//! artifact, which is checksummed.  The package verifies itself during
//! packaging.  The checksum is deterministic for the same clean
//! checkout.

#![allow(dead_code)]

#[cfg(test)]
mod release_build_tests;

use crate::lifecycle::{
    command_spec::DEFAULT_COMMAND_TIMEOUT, release_gates::run_bounded_gate_command,
};
use std::{error::Error, fmt, path::Path, process::Command, time::Duration};

/// The packaged artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageArtifact {
    pub crate_name: String,
    pub version: String,
    pub checksum: String,
    pub artifact_path: std::path::PathBuf,
}

/// Packaging failures.
#[derive(Debug)]
pub enum PackageError {
    DirtyCheckout {
        detail: String,
    },
    CommitMismatch {
        expected: String,
        actual: String,
    },
    Cargo {
        reason: String,
    },
    Artifact {
        path: std::path::PathBuf,
        reason: String,
    },
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirtyCheckout { detail } => {
                write!(formatter, "the checkout is not clean: {detail}")
            }
            Self::CommitMismatch { expected, actual } => write!(
                formatter,
                "the checkout HEAD {actual} does not match the manifest commit {expected}"
            ),
            Self::Cargo { reason } => write!(formatter, "cargo packaging failure: {reason}"),
            Self::Artifact { path, reason } => {
                write!(formatter, "artifact failure {}: {reason}", path.display())
            }
        }
    }
}
impl Error for PackageError {}

/// Build the locked package from the clean exact-SHA checkout.
pub fn build_locked_package(
    checkout: &Path,
    source_commit: &str,
) -> Result<PackageArtifact, PackageError> {
    let mut command = Command::new("cargo");
    command
        .args(["package", "--locked"])
        .current_dir(checkout)
        .env("CARGO_TERM_COLOR", "never");
    build_locked_package_with_command(checkout, source_commit, command, DEFAULT_COMMAND_TIMEOUT)
}

fn build_locked_package_with_command(
    checkout: &Path,
    source_commit: &str,
    command: Command,
    budget: Duration,
) -> Result<PackageArtifact, PackageError> {
    // 1. The checkout must be clean.
    let status = git_text(checkout, &["status", "--porcelain"]);
    if !status.is_empty() {
        return Err(PackageError::DirtyCheckout { detail: status });
    }
    // 2. The HEAD must be the exact manifest commit.
    let head = git_text(checkout, &["rev-parse", "HEAD"]);
    if head != source_commit {
        return Err(PackageError::CommitMismatch {
            expected: source_commit.to_owned(),
            actual: head,
        });
    }
    // 3. Package with locked dependencies; the package verifies itself.
    let run = run_bounded_gate_command("cargo package", command, budget);
    if !run.passed {
        return Err(PackageError::Cargo {
            reason: run.evidence,
        });
    }
    // 4. Locate the artifact and checksum it.
    let artifact_path = locate_artifact(checkout)?;
    let bytes = std::fs::read(&artifact_path).map_err(|error| PackageError::Artifact {
        path: artifact_path.clone(),
        reason: error.to_string(),
    })?;
    let checksum = sha256_hex(&bytes);
    let file_name = artifact_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // release-fixture-0.1.0.crate -> name/version.
    let (crate_name, version) = file_name
        .strip_suffix(".crate")
        .and_then(|stem| stem.rsplit_once('-'))
        .map(|(name, version)| (name.to_owned(), version.to_owned()))
        .unwrap_or_else(|| (file_name.clone(), String::new()));
    Ok(PackageArtifact {
        crate_name,
        version,
        checksum,
        artifact_path,
    })
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    if !output.status.success() {
        panic!("git {args:?}: {:?}", output);
    }
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn locate_artifact(checkout: &Path) -> Result<std::path::PathBuf, PackageError> {
    let target = checkout.join("target/package");
    if !target.is_dir() {
        return Err(PackageError::Artifact {
            path: target,
            reason: "the package directory does not exist".to_owned(),
        });
    }
    let candidates = std::fs::read_dir(&target)
        .map_err(|error| PackageError::Artifact {
            path: target.clone(),
            reason: error.to_string(),
        })?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().map(|ext| ext == "crate").unwrap_or(false))
        .collect::<Vec<_>>();
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| PackageError::Artifact {
            path: target,
            reason: "no .crate artifact was produced".to_owned(),
        })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // A compact SHA-256 for the artifact checksum (the artifact is
    // bounded and local; the FNV fallback is not used).
    let bit_length = (bytes.len() as u64) * 8;
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());
    for chunk in padded.chunks(64) {
        let mut w = [0_u32; 64];
        for (index, word) in chunk.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    state
        .iter()
        .map(|word| format!("{word:08x}"))
        .collect::<String>()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
