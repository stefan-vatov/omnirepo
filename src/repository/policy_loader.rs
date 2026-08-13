//! Canonical root repository policy loading.
//!
//! A destination repository may declare exactly `.omnirepo.yaml` at its
//! root (canon/architecture/configuration-authority.md).  Discovery checks
//! only that exact file: true no-file is absence; permission, symlink,
//! non-regular, competing-extension, legacy-authority, malformed,
//! unsupported-version, and invalid-policy cases are typed errors, never
//! absence or fallback.  Parsing reuses the strict configuration YAML
//! subset; no hooks, includes, or repository-controlled execution exist in
//! the YAML path.

#![allow(dead_code)]

use super::policy::{
    CommandPolicy, ManagedItemId, PolicyError, RepositoryPolicy, SchemaVersion, SelectionPolicy,
    VerificationCommand,
};
use crate::configuration::{YValue, parse_yaml_subset};
use std::{error::Error, fmt, fs, path::Path, path::PathBuf};

/// Canonical destination-root policy file name.
pub const POLICY_FILE_NAME: &str = ".omnirepo.yaml";
/// Competing alternate extension that must never load alongside the canon.
pub const COMPETING_FILE_NAME: &str = ".omnirepo.yml";
/// Legacy authority file that is an error rather than a fallback.
pub const LEGACY_FILE_NAME: &str = ".omni.yaml";

/// Presence outcome for the canonical root policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyPresence {
    /// No policy file exists; inference governs (convention until intent).
    Absent,
    /// The canonical file loaded into the validated repository policy.
    Present(RepositoryPolicy),
}

/// Typed policy-loading failures; never treated as absence.
#[derive(Debug)]
pub enum PolicyLoadError {
    /// The canonical path is not a regular file.
    NotRegular { path: PathBuf },
    /// The canonical path is a symlink or alias.
    Alias { path: PathBuf },
    /// A competing alternate extension exists alongside the canonical file.
    Competing { path: PathBuf, competitor: PathBuf },
    /// A legacy authority file exists; it is an error, not a fallback.
    LegacyAuthority { path: PathBuf },
    /// The canonical file cannot be read.
    Permission { path: PathBuf, reason: String },
    /// The canonical file is not valid subset YAML.
    Malformed { path: PathBuf, reason: String },
    /// The canonical file declares an unsupported schema version.
    UnsupportedVersion { path: PathBuf, version: u64 },
    /// The canonical file maps to an invalid repository policy.
    Invalid { path: PathBuf, error: PolicyError },
}

impl fmt::Display for PolicyLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegular { path } => {
                write!(
                    formatter,
                    "repository policy is not a regular file: {}",
                    path.display()
                )
            }
            Self::Alias { path } => write!(
                formatter,
                "repository policy must not be a symlink or alias: {}",
                path.display()
            ),
            Self::Competing { path, competitor } => write!(
                formatter,
                "competing repository policy {} alongside {}",
                competitor.display(),
                path.display()
            ),
            Self::LegacyAuthority { path } => write!(
                formatter,
                "legacy authority file {} is an error, not a fallback",
                path.display()
            ),
            Self::Permission { path, reason } => write!(
                formatter,
                "cannot read repository policy {}: {reason}",
                path.display()
            ),
            Self::Malformed { path, reason } => write!(
                formatter,
                "repository policy {} is malformed: {reason}",
                path.display()
            ),
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "repository policy {} uses unsupported version {version}",
                path.display()
            ),
            Self::Invalid { path, error } => write!(
                formatter,
                "repository policy {} is invalid: {error}",
                path.display()
            ),
        }
    }
}
impl Error for PolicyLoadError {}

/// Load the canonical root policy.  Side-effect free beyond reading the
/// exact canonical file (and probing the two forbidden siblings).
pub fn load_policy(root: &Path) -> Result<PolicyPresence, PolicyLoadError> {
    let canonical = root.join(POLICY_FILE_NAME);
    let competing = root.join(COMPETING_FILE_NAME);
    let legacy = root.join(LEGACY_FILE_NAME);
    let metadata = match fs::symlink_metadata(&canonical) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Absence is lawful only when no forbidden sibling exists.
            if legacy.exists() {
                return Err(PolicyLoadError::LegacyAuthority { path: legacy });
            }
            return Ok(PolicyPresence::Absent);
        }
        Err(error) => {
            return Err(PolicyLoadError::Permission {
                path: canonical,
                reason: error.to_string(),
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(PolicyLoadError::Alias { path: canonical });
    }
    if !metadata.is_file() {
        return Err(PolicyLoadError::NotRegular { path: canonical });
    }
    if competing.exists() {
        return Err(PolicyLoadError::Competing {
            path: canonical,
            competitor: competing,
        });
    }
    if legacy.exists() {
        return Err(PolicyLoadError::LegacyAuthority { path: legacy });
    }
    let content = fs::read(&canonical).map_err(|error| PolicyLoadError::Permission {
        path: canonical.clone(),
        reason: error.to_string(),
    })?;
    let text = String::from_utf8_lossy(&content);
    let value = parse_yaml_subset(&text).map_err(|error| PolicyLoadError::Malformed {
        path: canonical.clone(),
        reason: error.to_string(),
    })?;
    let policy = build_policy(&canonical, value)?;
    Ok(PolicyPresence::Present(policy))
}

fn map_get<'a>(map: &'a [(String, YValue)], key: &str) -> Option<&'a YValue> {
    map.iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value)
}

fn build_policy(path: &Path, value: YValue) -> Result<RepositoryPolicy, PolicyLoadError> {
    let YValue::Map(object) = value else {
        return Err(PolicyLoadError::Malformed {
            path: path.to_path_buf(),
            reason: "top level must be a mapping".to_owned(),
        });
    };
    let version_number = map_get(&object, "version")
        .and_then(YValue::as_u64)
        .ok_or_else(|| PolicyLoadError::Malformed {
            path: path.to_path_buf(),
            reason: "version must be an unsigned integer".to_owned(),
        })?;
    let version =
        SchemaVersion::new(version_number).map_err(|_| PolicyLoadError::UnsupportedVersion {
            path: path.to_path_buf(),
            version: version_number,
        })?;

    // Unknown fields are rejected: the policy schema is closed.
    for (key, _) in &object {
        if !matches!(
            key.as_str(),
            "version" | "all" | "allow" | "exclude" | "commands"
        ) {
            return Err(PolicyLoadError::Malformed {
                path: path.to_path_buf(),
                reason: format!("unknown repository policy field {key:?}"),
            });
        }
    }

    let has_selectors = ["all", "allow", "exclude"]
        .iter()
        .any(|key| map_get(&object, key).is_some());
    let selection = if has_selectors {
        let all = match map_get(&object, "all") {
            None => false,
            Some(YValue::String(value)) if value == "true" => true,
            Some(YValue::String(value)) if value == "false" => false,
            Some(_) => {
                return Err(PolicyLoadError::Malformed {
                    path: path.to_path_buf(),
                    reason: "all must be a boolean".to_owned(),
                });
            }
        };
        let allow = map_get(&object, "allow")
            .map(|value| load_ids(path, "allow", value))
            .transpose()?
            .unwrap_or_default();
        let exclude = map_get(&object, "exclude")
            .map(|value| load_ids(path, "exclude", value))
            .transpose()?
            .unwrap_or_default();
        SelectionPolicy::explicit(all, allow, exclude).map_err(|error| {
            PolicyLoadError::Invalid {
                path: path.to_path_buf(),
                error,
            }
        })?
    } else {
        SelectionPolicy::omitted()
    };

    let commands = match map_get(&object, "commands") {
        None => CommandPolicy::absent(),
        Some(YValue::List(items)) => {
            let mut commands = Vec::new();
            for item in items {
                let YValue::List(argv) = item else {
                    return Err(PolicyLoadError::Malformed {
                        path: path.to_path_buf(),
                        reason: "commands entries must be argv lists".to_owned(),
                    });
                };
                let argv = argv
                    .iter()
                    .map(|argument| {
                        argument.as_str().map(str::to_owned).ok_or_else(|| {
                            PolicyLoadError::Malformed {
                                path: path.to_path_buf(),
                                reason: "command arguments must be strings".to_owned(),
                            }
                        })
                    })
                    .collect::<Result<Vec<String>, PolicyLoadError>>()?;
                commands.push(VerificationCommand::new(argv).map_err(|error| {
                    PolicyLoadError::Invalid {
                        path: path.to_path_buf(),
                        error,
                    }
                })?);
            }
            CommandPolicy::present(commands).map_err(|error| PolicyLoadError::Invalid {
                path: path.to_path_buf(),
                error,
            })?
        }
        Some(_) => {
            return Err(PolicyLoadError::Malformed {
                path: path.to_path_buf(),
                reason: "commands must be a list".to_owned(),
            });
        }
    };

    RepositoryPolicy::new(version, selection, commands).map_err(|error| PolicyLoadError::Invalid {
        path: path.to_path_buf(),
        error,
    })
}

fn load_ids(
    path: &Path,
    field: &str,
    value: &YValue,
) -> Result<Vec<ManagedItemId>, PolicyLoadError> {
    let items = value.as_list().ok_or_else(|| PolicyLoadError::Malformed {
        path: path.to_path_buf(),
        reason: format!("{field} must be a list"),
    })?;
    items
        .iter()
        .map(|item| {
            let text = item.as_str().ok_or_else(|| PolicyLoadError::Malformed {
                path: path.to_path_buf(),
                reason: format!("{field} entries must be strings"),
            })?;
            ManagedItemId::parse(text).map_err(|error| PolicyLoadError::Invalid {
                path: path.to_path_buf(),
                error,
            })
        })
        .collect()
}
