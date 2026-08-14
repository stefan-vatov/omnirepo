//! The owner-selected supported-platform matrix.
//!
//! The .35 owner decision (APPROVED 2026-08-13): the first constitutional
//! release supports Linux and macOS on ordinary local filesystems
//! (ext-family on Linux, APFS on macOS).  Windows and network
//! filesystems are unsupported and fail closed before effects.  This
//! module translates the decision into explicit OS/filesystem/toolchain
//! jobs with capability detection, unsupported-case behavior, cache
//! isolation, and the repository-owned quality command.  No invented
//! platform is ever claimed.

#![allow(dead_code)]

#[cfg(test)]
mod platform_matrix_tests;

use std::{error::Error, fmt};

/// The supported operating systems.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Os {
    Linux,
    Mac,
    Windows,
}

/// The supported filesystem families.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Filesystem {
    ExtFamily,
    Apfs,
    Network,
}

/// The pinned toolchain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Toolchain {
    Rust186,
}

/// The required quality gates per platform job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateKind {
    Tests,
    Docs,
    Locked,
    Quality,
}

/// One explicit platform job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformJob {
    pub os: Os,
    pub filesystem: Filesystem,
    pub toolchain: Toolchain,
    pub required: &'static [GateKind],
}

impl PlatformJob {
    /// The isolated cache key for this job.
    pub fn cache_key(&self) -> String {
        format!("{:?}-{:?}-{:?}", self.os, self.filesystem, self.toolchain)
    }
}

/// One documented unsupported case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedCase {
    pub os: Os,
    pub filesystem: Filesystem,
    pub policy: &'static str,
}

/// Platform claim failures.
#[derive(Debug)]
pub enum PlatformError {
    Unsupported {
        os: Os,
        filesystem: Filesystem,
        policy: &'static str,
    },
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported {
                os,
                filesystem,
                policy,
            } => {
                write!(
                    formatter,
                    "{os:?} on {filesystem:?} is unsupported and fails closed: {policy}"
                )
            }
        }
    }
}
impl Error for PlatformError {}

/// Capability detection: is the pair supported per the owner decision?
pub fn capability_supported(os: Os, filesystem: Filesystem) -> bool {
    matches!(
        (os, filesystem),
        (Os::Linux, Filesystem::ExtFamily) | (Os::Mac, Filesystem::Apfs)
    )
}

/// The explicit supported-platform matrix.
///
/// Every supported platform runs the required tests, docs, locked gates,
/// and the repository-owned quality command on Rust 1.86.
pub fn supported_platform_matrix() -> Vec<PlatformJob> {
    let required: &[GateKind] = &[
        GateKind::Tests,
        GateKind::Docs,
        GateKind::Locked,
        GateKind::Quality,
    ];
    vec![
        PlatformJob {
            os: Os::Linux,
            filesystem: Filesystem::ExtFamily,
            toolchain: Toolchain::Rust186,
            required,
        },
        PlatformJob {
            os: Os::Mac,
            filesystem: Filesystem::Apfs,
            toolchain: Toolchain::Rust186,
            required,
        },
    ]
}

/// The documented unsupported cases: explicitly omitted from jobs and
/// failing closed before any effect.
pub fn unsupported_cases() -> Vec<UnsupportedCase> {
    vec![
        UnsupportedCase {
            os: Os::Windows,
            filesystem: Filesystem::Network,
            policy: "windows and network filesystems are unsupported and fail closed before effects",
        },
        UnsupportedCase {
            os: Os::Windows,
            filesystem: Filesystem::ExtFamily,
            policy: "windows is unsupported in the first constitutional release",
        },
        UnsupportedCase {
            os: Os::Linux,
            filesystem: Filesystem::Network,
            policy: "network filesystems are unsupported and fail closed before effects",
        },
        UnsupportedCase {
            os: Os::Mac,
            filesystem: Filesystem::Network,
            policy: "network filesystems are unsupported and fail closed before effects",
        },
    ]
}

/// Claim a platform for an operation.  An unsupported or invented pair
/// fails typed before any effect.
pub fn claim_platform(os: Os, filesystem: Filesystem) -> Result<(), PlatformError> {
    if capability_supported(os, filesystem) {
        Ok(())
    } else {
        let policy = unsupported_cases()
            .iter()
            .find(|case| case.os == os && case.filesystem == filesystem)
            .map(|case| case.policy)
            .unwrap_or("no invented platform is claimed");
        Err(PlatformError::Unsupported {
            os,
            filesystem,
            policy,
        })
    }
}
