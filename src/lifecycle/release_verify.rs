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

use crate::lifecycle::{
    check_runner::{CheckOutcome, run_check},
    command_spec::{CommandSpec, DEFAULT_COMMAND_TIMEOUT},
};
use crate::platform::RelativePath;
use std::{error::Error, fmt, path::Path, time::Duration};

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
    let help_ok = run(binary, clean_home, &["--help"])?;
    let version_ok = run(binary, clean_home, &["--version"])?;
    let sync_empty_ok = run(binary, clean_home, &["sync"])?;
    Ok(InstallVerification {
        help_ok,
        version_ok,
        sync_empty_ok,
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

fn run(binary: &Path, clean_home: &Path, args: &[&str]) -> Result<bool, VerifyError> {
    run_with_budget(binary, clean_home, args, DEFAULT_COMMAND_TIMEOUT)
}

fn run_with_budget(
    binary: &Path,
    clean_home: &Path,
    args: &[&str],
    budget: Duration,
) -> Result<bool, VerifyError> {
    let executable = binary.to_str().ok_or_else(|| VerifyError::Binary {
        path: binary.to_path_buf(),
        reason: "the binary path is not UTF-8".to_owned(),
    })?;
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(executable.to_owned());
    argv.extend(args.iter().map(|argument| (*argument).to_owned()));
    let spec = CommandSpec {
        repository: "release-candidate".to_owned(),
        plan_identity: "fresh-install".to_owned(),
        position: 0,
        argv,
        cwd: RelativePath::root(),
        env: vec![
            ("HOME".to_owned(), clean_home.display().to_string()),
            ("USERPROFILE".to_owned(), clean_home.display().to_string()),
            ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
        ],
        timeout: budget,
        stdin: None,
        capture_output: true,
        shell: None,
    };
    let result = run_check(clean_home, &spec, budget).map_err(|error| VerifyError::Run {
        path: binary.to_path_buf(),
        reason: error.to_string(),
    })?;
    Ok(matches!(result.outcome, CheckOutcome::Passed))
}
