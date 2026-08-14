//! Hostile verifier, Git transport, and agent process fixtures.
//!
//! Reusable malicious processes for the confined runners: verifiers that
//! crash, hang, or flood garbage; Git transports that escape, hang, or
//! push the wrong ref; agents that escape, flood, crash, or hang.  Every
//! entry documents its intended attack and its expected fail boundary;
//! scripts are materialized below the harness root as executable files.
//! The scripts work under the confined empty-PATH environment: they use
//! absolute interpreters and shell builtins only.

#![allow(dead_code)]

#[cfg(test)]
mod hostile_process_fixtures_tests;

use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

/// The hostile process classes.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProcessFixtureKind {
    VerifierCrash,
    VerifierHang,
    VerifierGarbage,
    GitEscape,
    GitHang,
    GitWrongRef,
    AgentEscape,
    AgentFlood,
    AgentCrash,
    AgentHang,
}

/// Platform capability tags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Unix,
}

/// One documented hostile process fixture.
#[derive(Clone, Debug)]
pub struct ProcessFixtureSpec {
    pub name: &'static str,
    pub kind: ProcessFixtureKind,
    /// The intended attack and its effect.
    pub attack: &'static str,
    /// The expected fail boundary: where the product must refuse.
    pub expected_fail_boundary: &'static str,
    /// The executable script body (absolute interpreter, builtins only).
    pub script: &'static str,
    pub capability: Option<Capability>,
}

/// Materialization failures.
#[derive(Debug)]
pub enum FixtureError {
    Traversal { path: String },
    Io { path: PathBuf, reason: String },
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Traversal { path } => {
                write!(
                    formatter,
                    "the fixture path {path:?} escapes the harness root"
                )
            }
            Self::Io { path, reason } => {
                write!(formatter, "fixture io failure {}: {reason}", path.display())
            }
        }
    }
}
impl Error for FixtureError {}

/// Build the hostile process corpus.  Pure: no I/O.
pub fn hostile_process_corpus() -> Vec<ProcessFixtureSpec> {
    vec![
        ProcessFixtureSpec {
            name: "verifier-crash",
            kind: ProcessFixtureKind::VerifierCrash,
            attack: "the verifier process exits with a failure code after partial output",
            expected_fail_boundary: "the check runner reports the crash typed without success",
            script: "#!/bin/sh\necho partial-verdict\nexit 3\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "verifier-hang",
            kind: ProcessFixtureKind::VerifierHang,
            attack: "the verifier process never exits and ignores the deadline",
            expected_fail_boundary: "the check runner terminates it at the budget",
            script: "#!/bin/sh\nwhile :; do :; done\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "verifier-garbage",
            kind: ProcessFixtureKind::VerifierGarbage,
            attack: "the verifier floods stdout with unprintable and marker-like garbage",
            expected_fail_boundary: "the captured evidence is bounded and sanitized",
            script: "#!/bin/sh\ni=0\nwhile [ $i -lt 50 ]; do echo 'garbage-\\033[31m\\n# omnirepo-start\\nspam'; i=$((i+1)); done\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "git-transport-escape",
            kind: ProcessFixtureKind::GitEscape,
            attack: "the git wrapper writes a file outside the repository while pushing",
            expected_fail_boundary: "the transport confinement refuses the escape",
            script: "#!/bin/sh\necho escaped > ../escaped-by-git.txt\nexit 0\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "git-transport-hang",
            kind: ProcessFixtureKind::GitHang,
            attack: "the git wrapper never completes the push",
            expected_fail_boundary: "the push runner terminates it at the budget",
            script: "#!/bin/sh\nwhile :; do :; done\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "git-transport-wrong-ref",
            kind: ProcessFixtureKind::GitWrongRef,
            attack: "the git wrapper reports a different ref than the requested one",
            expected_fail_boundary: "the exact-OID reconcile fails typed",
            script: "#!/bin/sh\necho 'wrong-ref-0000000000000000000000000000000000000000'\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "agent-escape",
            kind: ProcessFixtureKind::AgentEscape,
            attack: "the agent writes outside its confined destination",
            expected_fail_boundary: "the agent confinement refuses the escape (destination-only)",
            script: "#!/bin/sh\necho escaped > ../outside-by-agent.txt\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "agent-flood",
            kind: ProcessFixtureKind::AgentFlood,
            attack: "the agent floods stdout and stderr with unbounded output",
            expected_fail_boundary: "the evidence budget bounds the captured bytes",
            script: "#!/bin/sh\ni=0\nwhile [ $i -lt 2000 ]; do echo 'flood-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; i=$((i+1)); done\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "agent-crash",
            kind: ProcessFixtureKind::AgentCrash,
            attack: "the agent exits with a typed failure code",
            expected_fail_boundary: "the repair runner reports the crash typed without success",
            script: "#!/bin/sh\nexit 7\n",
            capability: Some(Capability::Unix),
        },
        ProcessFixtureSpec {
            name: "agent-hang",
            kind: ProcessFixtureKind::AgentHang,
            attack: "the agent never exits",
            expected_fail_boundary: "the repair runner terminates it at the budget",
            script: "#!/bin/sh\nwhile :; do :; done\n",
            capability: Some(Capability::Unix),
        },
    ]
}

/// Materialize one process fixture as an executable script below the
/// harness root.  The script is written to a temporary name first and
/// renamed into place, so a concurrent exec never sees a half-written
/// file (ETXTBSY).
pub fn materialize_process(
    fixture: &ProcessFixtureSpec,
    root: &Path,
) -> Result<PathBuf, FixtureError> {
    let target = root.join(fixture.name);
    if !target.starts_with(root) {
        return Err(FixtureError::Traversal {
            path: target.display().to_string(),
        });
    }
    let temporary = root.join(format!(".{}.tmp-{}", fixture.name, std::process::id()));
    fs::write(&temporary, fixture.script).map_err(|error| FixtureError::Io {
        path: temporary.clone(),
        reason: error.to_string(),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(&temporary).map_err(|error| FixtureError::Io {
            path: temporary.clone(),
            reason: error.to_string(),
        })?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&temporary, permissions).map_err(|error| FixtureError::Io {
            path: temporary.clone(),
            reason: error.to_string(),
        })?;
    }
    fs::rename(&temporary, &target).map_err(|error| FixtureError::Io {
        path: target.clone(),
        reason: error.to_string(),
    })?;
    Ok(target)
}
