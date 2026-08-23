//! The supported-platform matrix.
//!
//! Linux is supported on every filesystem. macOS is supported on APFS.
//! Windows and non-APFS macOS filesystems are unsupported and fail closed
//! before effects. This module translates that policy into explicit
//! OS/filesystem/toolchain jobs with capability detection, unsupported-case
//! behavior, cache isolation, and the repository-owned quality command. No
//! invented platform is ever claimed.

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
    Linux,
    Apfs,
    Other,
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
        (Os::Linux, Filesystem::Linux) | (Os::Mac, Filesystem::Apfs)
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
            filesystem: Filesystem::Linux,
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
            filesystem: Filesystem::Other,
            policy: "windows is unsupported and fails closed before effects",
        },
        UnsupportedCase {
            os: Os::Mac,
            filesystem: Filesystem::Other,
            policy: "macOS filesystems other than APFS are unsupported and fail closed before effects",
        },
    ]
}

/// The canonical CI job names for the declared matrix.  The drift
/// contract ties these to the live workflow jobs.
pub fn ci_job_names() -> Vec<&'static str> {
    vec!["quality", "msrv", "macos-quality"]
}

/// Render the concise support/evidence report from the declared matrix
/// (the single source of truth).  The committed report must match this
/// rendering exactly.  The format is the canonical pretty JSON
/// (2-space indent) that the repository formatter enforces.
pub fn platform_evidence() -> String {
    let mut lines = vec![
        "{".to_owned(),
        "  \"schema\": \"omnirepo.platform-evidence.v1\",".to_owned(),
        "  \"toolchain\": \"1.86\",".to_owned(),
        "  \"supported\": [".to_owned(),
    ];
    let matrix = supported_platform_matrix();
    for (index, job) in matrix.iter().enumerate() {
        let comma = if index + 1 < matrix.len() { "," } else { "" };
        let jobs = match job.os {
            Os::Linux => vec!["quality", "msrv"],
            Os::Mac => vec!["macos-quality"],
            Os::Windows => Vec::new(),
        };
        lines.push(format!(
            "    {{\n      \"os\": \"{:?}\",\n      \"filesystem\": \"{:?}\",\n      \"jobs\": [",
            job.os, job.filesystem
        ));
        for (job_index, name) in jobs.iter().enumerate() {
            let job_comma = if job_index + 1 < jobs.len() { "," } else { "" };
            lines.push(format!("        \"{name}\"{job_comma}"));
        }
        lines.push(format!(
            "      ],\n      \"cache\": \"{}\"\n    }}{comma}",
            job.cache_key()
        ));
    }
    lines.push("  ],".to_owned());
    lines.push("  \"capability_skips\": [".to_owned());
    for (index, case) in unsupported_cases().iter().enumerate() {
        let comma = if index + 1 < unsupported_cases().len() {
            ","
        } else {
            ""
        };
        lines.push(format!(
            "    {{\n      \"os\": \"{:?}\",\n      \"filesystem\": \"{:?}\",\n      \"policy\": \"fail-closed\"\n    }}{comma}",
            case.os, case.filesystem
        ));
    }
    lines.push("  ]".to_owned());
    lines.push("}".to_owned());
    lines.join("\n")
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
