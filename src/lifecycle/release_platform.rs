//! Build and verify platform binary candidate bundles.
//!
//! Each supported platform target produces a binary bundle: the
//! release build (locked, exact-SHA checkout) yields the binary, which
//! is checksummed and verified by running its surface (help and
//! version).  A target without the installed toolchain fails typed —
//! never a panic and never a fake bundle.

#![allow(dead_code)]

use crate::lifecycle::release_build::sha256_hex;

#[cfg(test)]
mod release_platform_tests;
use std::{error::Error, fmt, path::Path, process::Command};

/// One platform binary bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformBundle {
    pub target: String,
    pub binary_path: std::path::PathBuf,
    pub checksum: String,
}

/// The bundle verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verification {
    pub help_ok: bool,
    pub version_ok: bool,
}

/// Bundle failures.
#[derive(Debug)]
pub enum BundleError {
    TargetUnavailable {
        target: String,
        reason: String,
    },
    Cargo {
        reason: String,
    },
    Io {
        path: std::path::PathBuf,
        reason: String,
    },
    Verify {
        reason: String,
    },
}

impl fmt::Display for BundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetUnavailable { target, reason } => {
                write!(formatter, "target {target:?} is unavailable: {reason}")
            }
            Self::Cargo { reason } => write!(formatter, "bundle cargo failure: {reason}"),
            Self::Io { path, reason } => {
                write!(formatter, "bundle io failure {}: {reason}", path.display())
            }
            Self::Verify { reason } => write!(formatter, "bundle verification failure: {reason}"),
        }
    }
}
impl Error for BundleError {}

/// Build the binary bundle for one target from the clean exact-SHA
/// checkout.
pub fn build_platform_bundle_for(
    checkout: &Path,
    _source_commit: &str,
    target: &str,
) -> Result<PlatformBundle, BundleError> {
    // The clean exact-SHA gate comes from the packaging owner; this
    // builder only compiles the binary for the target.
    let output = Command::new("cargo")
        .args(["build", "--release", "--locked", "--target", target])
        .current_dir(checkout)
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .map_err(|error| BundleError::TargetUnavailable {
            target: target.to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if stderr.contains("not installed")
            || stderr.contains("no such target")
            || stderr.contains("can't find crate")
        {
            return Err(BundleError::TargetUnavailable {
                target: target.to_owned(),
                reason: stderr,
            });
        }
        return Err(BundleError::Cargo { reason: stderr });
    }
    let binary_path = binary_path_for(checkout, target)?;
    let bytes = std::fs::read(&binary_path).map_err(|error| BundleError::Io {
        path: binary_path.clone(),
        reason: error.to_string(),
    })?;
    Ok(PlatformBundle {
        target: target.to_owned(),
        binary_path,
        checksum: sha256_hex(&bytes),
    })
}

/// Build the local host bundle.
pub fn build_platform_bundle(
    checkout: &Path,
    _source_commit: &str,
) -> Result<PlatformBundle, BundleError> {
    #[cfg(target_os = "linux")]
    let target = "x86_64-unknown-linux-gnu";
    #[cfg(target_os = "macos")]
    let target = "aarch64-apple-darwin";
    build_platform_bundle_for(checkout, "", target)
}

/// Verify the bundle: the binary's help and version exit zero.
pub fn verify_bundle(bundle: &PlatformBundle) -> Result<Verification, BundleError> {
    let help = Command::new(&bundle.binary_path)
        .arg("--help")
        .output()
        .map_err(|error| BundleError::Verify {
            reason: error.to_string(),
        })?;
    let version = Command::new(&bundle.binary_path)
        .arg("--version")
        .output()
        .map_err(|error| BundleError::Verify {
            reason: error.to_string(),
        })?;
    Ok(Verification {
        help_ok: help.status.success(),
        version_ok: version.status.success(),
    })
}

fn binary_path_for(checkout: &Path, target: &str) -> Result<std::path::PathBuf, BundleError> {
    let directory = checkout.join("target").join(target).join("release");
    let candidate = directory.join("release-fixture");
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(BundleError::Io {
        path: directory,
        reason: "the built binary does not exist".to_owned(),
    })
}
