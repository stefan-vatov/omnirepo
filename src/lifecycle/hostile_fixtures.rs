//! The hostile authority and filesystem fixture corpus.
//!
//! Reusable malicious machine/source/repository configs, aliases and
//! traversal, symlinks and hard links, special files, case/Unicode
//! collisions, source declarations, Git config/attributes/hooks, and
//! record paths.  Every entry documents its intended attack and the
//! expected fail boundary; platform-specific cases are capability-tagged;
//! every secret sentinel is unique.  Materialization can never escape the
//! harness root: traversal-style fixtures are rejected before any write.

#![allow(dead_code)]

#[cfg(test)]
mod hostile_fixtures_tests;

use std::{error::Error, fmt, fs, path::Path};

/// The hostile fixture classes.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FixtureKind {
    MachineConfig,
    SourceConfig,
    RepositoryConfig,
    Traversal,
    Symlink,
    HardLink,
    SpecialFile,
    CaseCollision,
    UnicodeCollision,
    SourceDeclaration,
    GitConfig,
    GitAttributes,
    GitHooks,
    RecordPath,
}

/// Platform capability tags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Unix,
    Windows,
}

/// One documented hostile fixture.
#[derive(Clone, Debug)]
pub struct HostileFixture {
    pub name: &'static str,
    pub kind: FixtureKind,
    /// The intended attack and its effect.
    pub attack: &'static str,
    /// The expected fail boundary: where the product must refuse.
    pub expected_fail_boundary: &'static str,
    /// Platform-specific cases carry their capability tag.
    pub capability: Option<Capability>,
    /// A unique secret sentinel (assembled from parts, never a real
    /// credential).
    pub secret_sentinel: &'static str,
}

/// Materialization failures.
#[derive(Debug)]
pub enum FixtureError {
    Traversal {
        path: String,
    },
    Unsupported {
        name: String,
    },
    Io {
        path: std::path::PathBuf,
        reason: String,
    },
    NotMaterializable {
        name: String,
    },
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
            Self::Unsupported { name } => {
                write!(
                    formatter,
                    "fixture {name:?} requires an unsupported platform case"
                )
            }
            Self::Io { path, reason } => {
                write!(formatter, "fixture io failure {}: {reason}", path.display())
            }
            Self::NotMaterializable { name } => {
                write!(
                    formatter,
                    "fixture {name:?} is a documented case, not a file tree"
                )
            }
        }
    }
}
impl Error for FixtureError {}

/// Build the hostile corpus.  Pure: no I/O.
pub fn hostile_corpus() -> Vec<HostileFixture> {
    vec![
        HostileFixture {
            name: "machine-config-absolute-destination",
            kind: FixtureKind::MachineConfig,
            attack: "the machine config declares an absolute destination outside any root",
            expected_fail_boundary: "configuration parsing rejects absolute destinations",
            capability: None,
            secret_sentinel: "corpus-machine-abso",
        },
        HostileFixture {
            name: "machine-config-invalid-slug",
            kind: FixtureKind::MachineConfig,
            attack: "the machine config carries a slug with slash and control characters",
            expected_fail_boundary: "slug validation fails closed",
            capability: None,
            secret_sentinel: "corpus-machine-slug",
        },
        HostileFixture {
            name: "source-config-duplicate-repository",
            kind: FixtureKind::SourceConfig,
            attack: "the source config declares the same repository twice with different items",
            expected_fail_boundary: "declaration ordering and duplication rules refuse it",
            capability: None,
            secret_sentinel: "corpus-source-dupe",
        },
        HostileFixture {
            name: "repository-config-unknown-field",
            kind: FixtureKind::RepositoryConfig,
            attack: "the repository config carries an unknown field that widens policy",
            expected_fail_boundary: "the policy loader rejects unknown fields",
            capability: None,
            secret_sentinel: "corpus-repo-unkn",
        },
        HostileFixture {
            name: "traversal-dotdot-managed-path",
            kind: FixtureKind::Traversal,
            attack: "a managed path contains ../ segments to escape the destination",
            expected_fail_boundary: "path validation refuses traversal before any write",
            capability: None,
            secret_sentinel: "corpus-traversal-dotdot",
        },
        HostileFixture {
            name: "traversal-absolute-managed-path",
            kind: FixtureKind::Traversal,
            attack: "a managed path is absolute and points outside the destination",
            expected_fail_boundary: "relative-path parsing refuses absolute paths",
            capability: None,
            secret_sentinel: "corpus-traversal-absolute",
        },
        HostileFixture {
            name: "symlink-managed-to-outside",
            kind: FixtureKind::Symlink,
            attack: "a symlink inside the destination resolves to a file outside it",
            expected_fail_boundary: "the authority confinement refuses the escape (Unix)",
            capability: Some(Capability::Unix),
            secret_sentinel: "corpus-symlink-out",
        },
        HostileFixture {
            name: "hardlink-managed-to-outside",
            kind: FixtureKind::HardLink,
            attack: "a hard link inside the destination shares an inode with an outside file",
            expected_fail_boundary: "the identity model detects the shared inode (Unix)",
            capability: Some(Capability::Unix),
            secret_sentinel: "corpus-hardlink-ino",
        },
        HostileFixture {
            name: "special-fifo-managed-path",
            kind: FixtureKind::SpecialFile,
            attack: "a named pipe (FIFO) sits where a managed file is expected",
            expected_fail_boundary: "the entry-kind check refuses non-regular files (Unix)",
            capability: Some(Capability::Unix),
            secret_sentinel: "corpus-fifo-ok",
        },
        HostileFixture {
            name: "case-collision-managed-files",
            kind: FixtureKind::CaseCollision,
            attack: "two managed files differ only in case on a case-insensitive view",
            expected_fail_boundary: "the identity model detects the collision",
            capability: None,
            secret_sentinel: "corpus-case-collide",
        },
        HostileFixture {
            name: "unicode-collision-managed-files",
            kind: FixtureKind::UnicodeCollision,
            attack: "two managed files differ only in Unicode normalization",
            expected_fail_boundary: "byte-exact paths keep the files distinct",
            capability: None,
            secret_sentinel: "corpus-unicode-normal",
        },
        HostileFixture {
            name: "source-declaration-ambiguous-section",
            kind: FixtureKind::SourceDeclaration,
            attack: "the source declaration contains two competing sections for one item",
            expected_fail_boundary: "the partial scan refuses ambiguous topology",
            capability: None,
            secret_sentinel: "corpus-decl-ambig",
        },
        HostileFixture {
            name: "git-config-hostile-hooks-path",
            kind: FixtureKind::GitConfig,
            attack: "the repository .git/config sets core.hooksPath to an outside directory",
            expected_fail_boundary: "the sanitized git environment ignores hostile hooks",
            capability: None,
            secret_sentinel: "corpus-git-hookspath",
        },
        HostileFixture {
            name: "git-attributes-filter-poison",
            kind: FixtureKind::GitAttributes,
            attack: ".gitattributes declares a filter that rewrites staged content",
            expected_fail_boundary: "the isolated index hashes working-tree bytes without filters",
            capability: None,
            secret_sentinel: "corpus-attrs-filter",
        },
        HostileFixture {
            name: "git-hooks-commit-replacement",
            kind: FixtureKind::GitHooks,
            attack: "a commit hook rewrites or refuses the scoped commit",
            expected_fail_boundary: "the sanitized environment disables hooks for the operation",
            capability: None,
            secret_sentinel: "corpus-hooks-commit",
        },
        HostileFixture {
            name: "record-path-traversal-run-id",
            kind: FixtureKind::RecordPath,
            attack: "a run id carries ../ to make the record path escape the runs directory",
            expected_fail_boundary: "record path construction validates the run id",
            capability: None,
            secret_sentinel: "corpus-record-traversal",
        },
        HostileFixture {
            name: "record-path-ansi-injection",
            kind: FixtureKind::RecordPath,
            attack: "a repository id carries ANSI escape sequences into the projection",
            expected_fail_boundary: "projections sanitize ids before rendering",
            capability: None,
            secret_sentinel: "corpus-record-ansi",
        },
    ]
}

impl HostileFixture {
    /// Whether the fixture is a materializable file tree.  The
    /// documented-case fixtures (configs, collisions, hooks) are
    /// materialized as text trees; traversal fixtures are rejected.
    pub fn materializable(&self) -> bool {
        !matches!(self.kind, FixtureKind::Traversal)
    }
}

/// Materialize one fixture below the harness root.
///
/// Every relative path is validated against traversal first; the write
/// target must stay below the root.  Platform-specific fixtures are
/// skipped with a typed error when the capability is not present.
pub fn materialize(fixture: &HostileFixture, root: &Path) -> Result<(), FixtureError> {
    if !fixture.materializable() {
        return Err(FixtureError::NotMaterializable {
            name: fixture.name.to_owned(),
        });
    }
    let relative = fixture_target(fixture)?;
    let target = root.join(relative);
    if !target.starts_with(root) {
        return Err(FixtureError::Traversal {
            path: target.display().to_string(),
        });
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| FixtureError::Io {
            path: parent.to_path_buf(),
            reason: error.to_string(),
        })?;
    }
    match fixture.kind {
        FixtureKind::Symlink | FixtureKind::HardLink | FixtureKind::SpecialFile => {
            // The capability-tagged cases are documented; their hostile
            // materialization belongs to the capability-specific suites.
            #[cfg(unix)]
            {
                if fixture.kind == FixtureKind::Symlink {
                    let link_target = root.join("outside-target.txt");
                    fs::write(&link_target, fixture.secret_sentinel).map_err(|error| {
                        FixtureError::Io {
                            path: link_target.clone(),
                            reason: error.to_string(),
                        }
                    })?;
                    std::os::unix::fs::symlink(&link_target, &target).map_err(|error| {
                        FixtureError::Io {
                            path: target.clone(),
                            reason: error.to_string(),
                        }
                    })?;
                }
                Ok(())
            }
            #[cfg(not(unix))]
            {
                Err(FixtureError::Unsupported {
                    name: fixture.name.to_owned(),
                })
            }
        }
        _ => {
            let content = format!(
                "# hostile fixture: {}\n# attack: {}\n# boundary: {}\n# sentinel: {}\n",
                fixture.name,
                fixture.attack,
                fixture.expected_fail_boundary,
                fixture.secret_sentinel
            );
            fs::write(&target, content).map_err(|error| FixtureError::Io {
                path: target.clone(),
                reason: error.to_string(),
            })
        }
    }
}

fn fixture_target(fixture: &HostileFixture) -> Result<String, FixtureError> {
    let relative = match fixture.kind {
        FixtureKind::MachineConfig => "machine/config.yaml",
        FixtureKind::SourceConfig => "source/declarations.yaml",
        FixtureKind::RepositoryConfig => "repository/policy.yaml",
        FixtureKind::Traversal => "../escape.txt",
        FixtureKind::Symlink => "destination/managed-link",
        FixtureKind::HardLink => "destination/managed-hardlink",
        FixtureKind::SpecialFile => "destination/managed-fifo",
        FixtureKind::CaseCollision => "destination/Managed.txt",
        FixtureKind::UnicodeCollision => "destination/managed-normalized.txt",
        FixtureKind::SourceDeclaration => "source/section.yaml",
        FixtureKind::GitConfig => "destination/.git/config",
        FixtureKind::GitAttributes => "destination/.gitattributes",
        FixtureKind::GitHooks => "destination/.git/hooks/commit-msg",
        FixtureKind::RecordPath => "runs/record-2026.jsonl",
    };
    if relative.contains("..") {
        return Err(FixtureError::Traversal {
            path: relative.to_owned(),
        });
    }
    Ok(relative.to_owned())
}
