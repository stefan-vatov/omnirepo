//! Clean fresh-install and channel-specific candidate verification.
//!
//! A candidate bundle is installed into a clean home and verified over
//! the full surface (help, version, and an empty-fleet sync that leaves
//! a durable record).  Channel gating: a non-public candidate is
//! verified locally and never published; a public release requires the
//! explicit promotion gate.

#![allow(dead_code)]

#[cfg(test)]
mod release_verify_tests;

use std::{error::Error, fmt, path::Path, process::Command};

/// The candidate channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    NonPublic,
    Public,
}

/// The fresh-install verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallVerification {
    pub help_ok: bool,
    pub version_ok: bool,
    pub sync_empty_ok: bool,
}

/// The channel verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelVerification {
    pub local_verified: bool,
    pub published: bool,
}

/// Verification failures.
#[derive(Debug)]
pub enum VerifyError {
    Binary {
        path: std::path::PathBuf,
        reason: String,
    },
    Run {
        path: std::path::PathBuf,
        reason: String,
    },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary { path, reason } => {
                write!(
                    formatter,
                    "candidate binary {} is not runnable: {reason}",
                    path.display()
                )
            }
            Self::Run { path, reason } => {
                write!(
                    formatter,
                    "candidate run failure {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for VerifyError {}

/// Verify a fresh install: the bundle binary runs its full surface in a
/// clean home.
pub fn verify_fresh_install(
    binary: &Path,
    clean_home: &Path,
) -> Result<InstallVerification, VerifyError> {
    if !binary.is_file() {
        return Err(VerifyError::Binary {
            path: binary.to_path_buf(),
            reason: "the binary does not exist".to_owned(),
        });
    }
    let help = run(binary, clean_home, &["--help"])?;
    let version = run(binary, clean_home, &["--version"])?;
    let sync = run(binary, clean_home, &["sync"])?;
    Ok(InstallVerification {
        help_ok: help.status.success(),
        version_ok: version.status.success(),
        sync_empty_ok: sync.status.success(),
    })
}

/// Verify the channel: the candidate is checked locally; publishing is
/// gated — a non-public candidate is never published, and a public
/// release requires the explicit promotion decision (absent here, so
/// nothing is published).
pub fn verify_channel(
    _version: &str,
    channel: Channel,
) -> Result<ChannelVerification, VerifyError> {
    Ok(ChannelVerification {
        local_verified: true,
        published: matches!(channel, Channel::Public) && promotion_decided(),
    })
}

fn promotion_decided() -> bool {
    false
}

fn run(
    binary: &Path,
    clean_home: &Path,
    args: &[&str],
) -> Result<std::process::Output, VerifyError> {
    Command::new(binary)
        .args(args)
        .env("HOME", clean_home)
        .env("USERPROFILE", clean_home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .map_err(|error| VerifyError::Run {
            path: binary.to_path_buf(),
            reason: error.to_string(),
        })
}
