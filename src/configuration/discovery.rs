//! Canonical machine-config discovery and no-alternate-authority loading.
//!
//! The machine configuration lives at exactly `<HOME>/.omnirepo/config.yaml`
//! (YAML version 1).  Discovery reads only that path: no `--config`
//! substitutes, no working-directory scanning, no network, and no destination
//! access.  Absence is a distinct, lawful state (inference governs until a
//! repository policy exists); every other outcome is a typed pre-effect
//! error.  Parsing uses the strict repository-owned YAML subset (the runtime
//! dependency surface is frozen to Clap only).

#![allow(dead_code)]

use super::yaml_subset::{YValue, parse_yaml_subset};
use super::{
    AbsolutePath, AgentKind, ConfigurationError, DestinationRepository, MachineConcurrency,
    MachineConfiguration, RepairControls, RepositoryId, RepositoryTag, SchemaVersion, SourceId,
    SourceLocation, SourceReference,
};
use std::{error::Error, fmt, fs, path::Path, path::PathBuf};

/// Canonical machine configuration directory below HOME.
pub const CONFIG_DIRECTORY: &str = ".omnirepo";
/// Canonical machine configuration file name.
pub const CONFIG_FILE_NAME: &str = "config.yaml";

/// Discovery outcome for the canonical configuration path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Discovery {
    /// No configuration file exists; inference governs until policy exists.
    Absent,
    /// The canonical file loaded into the immutable machine authority.
    Present(MachineConfiguration),
}

/// Typed pre-effect discovery failures.
#[derive(Debug)]
pub enum DiscoveryError {
    /// HOME is not an absolute directory.
    HomeUnavailable { reason: String },
    /// The canonical path exists but is not a regular file (directory, fifo).
    NotRegular { path: PathBuf },
    /// The canonical path is a symlink or alias; authorities are never
    /// reached through indirection.
    Alias { path: PathBuf },
    /// The canonical file cannot be read.
    Permission { path: PathBuf, reason: String },
    /// The canonical file is not valid subset YAML.
    Malformed { path: PathBuf, reason: String },
    /// The canonical file declares an unsupported schema version.
    UnsupportedVersion { path: PathBuf, version: u64 },
    /// The canonical file maps to an invalid machine configuration.
    Invalid {
        path: PathBuf,
        error: ConfigurationError,
    },
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeUnavailable { reason } => {
                write!(formatter, "machine-config home is unavailable: {reason}")
            }
            Self::NotRegular { path } => {
                write!(
                    formatter,
                    "machine config is not a regular file: {}",
                    path.display()
                )
            }
            Self::Alias { path } => write!(
                formatter,
                "machine config must not be a symlink or alias: {}",
                path.display()
            ),
            Self::Permission { path, reason } => write!(
                formatter,
                "cannot read machine config {}: {reason}",
                path.display()
            ),
            Self::Malformed { path, reason } => write!(
                formatter,
                "machine config {} is malformed: {reason}",
                path.display()
            ),
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "machine config {} uses unsupported version {version}",
                path.display()
            ),
            Self::Invalid { path, error } => write!(
                formatter,
                "machine config {} is invalid: {error}",
                path.display()
            ),
        }
    }
}
impl Error for DiscoveryError {}

/// Canonical discovery path below a home.
pub fn canonical_config_path(home: &Path) -> PathBuf {
    home.join(CONFIG_DIRECTORY).join(CONFIG_FILE_NAME)
}

/// Discover and load the canonical machine configuration.
///
/// Side-effect free beyond reading the single canonical file; never touches
/// the network, the working directory, or any destination repository.
pub fn discover(home: &Path) -> Result<Discovery, DiscoveryError> {
    if !home.is_absolute() || !home.is_dir() {
        return Err(DiscoveryError::HomeUnavailable {
            reason: "HOME is not an absolute directory".to_owned(),
        });
    }
    let path = canonical_config_path(home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Discovery::Absent);
        }
        Err(error) => {
            return Err(DiscoveryError::Permission {
                path,
                reason: error.to_string(),
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(DiscoveryError::Alias { path });
    }
    if !metadata.is_file() {
        return Err(DiscoveryError::NotRegular { path });
    }
    let content = fs::read(&path).map_err(|error| DiscoveryError::Permission {
        path: path.clone(),
        reason: error.to_string(),
    })?;
    let text = String::from_utf8_lossy(&content);
    let value = parse_yaml_subset(&text).map_err(|error| DiscoveryError::Malformed {
        path: path.clone(),
        reason: error.to_string(),
    })?;
    let config = build_configuration(&path, value)?;
    Ok(Discovery::Present(config))
}

fn map_get<'a>(map: &'a [(String, YValue)], key: &str) -> Option<&'a YValue> {
    map.iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value)
}

fn build_configuration(path: &Path, value: YValue) -> Result<MachineConfiguration, DiscoveryError> {
    let YValue::Map(object) = value else {
        return Err(DiscoveryError::Malformed {
            path: path.to_path_buf(),
            reason: "top level must be a mapping".to_owned(),
        });
    };
    let version_number = map_get(&object, "version")
        .and_then(YValue::as_u64)
        .ok_or_else(|| DiscoveryError::Malformed {
            path: path.to_path_buf(),
            reason: "version must be an unsigned integer".to_owned(),
        })?;
    let version = SchemaVersion::new(u16::try_from(version_number).map_err(|_| {
        DiscoveryError::UnsupportedVersion {
            path: path.to_path_buf(),
            version: version_number,
        }
    })?)
    .map_err(|_| DiscoveryError::UnsupportedVersion {
        path: path.to_path_buf(),
        version: version_number,
    })?;

    // The machine schema is closed: unknown fields — including any
    // destination-policy or ad-hoc authority field — are rejected.
    for (key, _) in &object {
        if !matches!(
            key.as_str(),
            "version" | "repositories" | "sources" | "cache_root" | "concurrency" | "repair"
        ) {
            return Err(DiscoveryError::Malformed {
                path: path.to_path_buf(),
                reason: format!("unknown machine configuration field {key:?}"),
            });
        }
    }

    let repositories = map_get(&object, "repositories")
        .map(|value| load_repositories(path, value))
        .transpose()?
        .unwrap_or_default();
    let sources = map_get(&object, "sources")
        .map(|value| load_sources(path, value))
        .transpose()?
        .unwrap_or_default();
    let cache_root = match map_get(&object, "cache_root") {
        None | Some(YValue::Null) => None,
        Some(YValue::String(text)) => {
            Some(
                AbsolutePath::parse(text).map_err(|error| DiscoveryError::Invalid {
                    path: path.to_path_buf(),
                    error,
                })?,
            )
        }
        Some(_) => {
            return Err(DiscoveryError::Malformed {
                path: path.to_path_buf(),
                reason: "cache_root must be a string".to_owned(),
            });
        }
    };
    let concurrency = match map_get(&object, "concurrency") {
        None => MachineConcurrency::new(
            super::DEFAULT_MAX_REPOSITORIES,
            super::DEFAULT_MAX_CHILD_WORK,
        )
        .expect("defaults are valid"),
        Some(YValue::Map(map)) => {
            let max_repositories = map_get(map, "max_repositories")
                .and_then(YValue::as_u64)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "concurrency.max_repositories must be an unsigned integer".to_owned(),
                })?;
            let max_child_work = map_get(map, "max_child_work")
                .and_then(YValue::as_u64)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "concurrency.max_child_work must be an unsigned integer".to_owned(),
                })?;
            MachineConcurrency::new(
                u16::try_from(max_repositories).map_err(|_| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "max_repositories exceeds the supported range".to_owned(),
                })?,
                u16::try_from(max_child_work).map_err(|_| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "max_child_work exceeds the supported range".to_owned(),
                })?,
            )
            .map_err(|error| DiscoveryError::Invalid {
                path: path.to_path_buf(),
                error,
            })?
        }
        Some(_) => {
            return Err(DiscoveryError::Malformed {
                path: path.to_path_buf(),
                reason: "concurrency must be a mapping".to_owned(),
            });
        }
    };
    let repair = match map_get(&object, "repair") {
        None => RepairControls::new(Vec::new(), super::DEFAULT_REPAIR_ATTEMPTS)
            .expect("defaults are valid"),
        Some(YValue::Map(map)) => {
            let priority = map_get(map, "priority")
                .map(|value| {
                    let items = value.as_list().ok_or_else(|| DiscoveryError::Malformed {
                        path: path.to_path_buf(),
                        reason: "repair.priority must be a list".to_owned(),
                    })?;
                    items
                        .iter()
                        .map(|item| {
                            let name = item.as_str().ok_or_else(|| DiscoveryError::Malformed {
                                path: path.to_path_buf(),
                                reason: "repair.priority entries must be strings".to_owned(),
                            })?;
                            agent_kind(name).ok_or_else(|| DiscoveryError::Malformed {
                                path: path.to_path_buf(),
                                reason: format!("unknown repair agent kind {name:?}"),
                            })
                        })
                        .collect::<Result<Vec<_>, DiscoveryError>>()
                })
                .transpose()?
                .unwrap_or_default();
            let max_attempts = map_get(map, "max_attempts")
                .and_then(YValue::as_u64)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "repair.max_attempts must be an unsigned integer".to_owned(),
                })?;
            RepairControls::new(
                priority,
                u8::try_from(max_attempts).map_err(|_| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "max_attempts exceeds the supported range".to_owned(),
                })?,
            )
            .map_err(|error| DiscoveryError::Invalid {
                path: path.to_path_buf(),
                error,
            })?
        }
        Some(_) => {
            return Err(DiscoveryError::Malformed {
                path: path.to_path_buf(),
                reason: "repair must be a mapping".to_owned(),
            });
        }
    };

    MachineConfiguration::new(
        version,
        repositories,
        sources,
        cache_root,
        concurrency,
        repair,
    )
    .map_err(|error| DiscoveryError::Invalid {
        path: path.to_path_buf(),
        error,
    })
}

fn load_repositories(
    path: &Path,
    value: &YValue,
) -> Result<Vec<DestinationRepository>, DiscoveryError> {
    let items = value.as_list().ok_or_else(|| DiscoveryError::Malformed {
        path: path.to_path_buf(),
        reason: "repositories must be a list".to_owned(),
    })?;
    items
        .iter()
        .map(|item| {
            let YValue::Map(map) = item else {
                return Err(DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "repository entries must be mappings".to_owned(),
                });
            };
            let id = map_get(map, "id")
                .and_then(YValue::as_str)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "repository.id must be a string".to_owned(),
                })
                .and_then(|value| {
                    RepositoryId::parse(value).map_err(|error| DiscoveryError::Invalid {
                        path: path.to_path_buf(),
                        error,
                    })
                })?;
            let path_value = map_get(map, "path")
                .and_then(YValue::as_str)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "repository.path must be a string".to_owned(),
                })?;
            let repository_path =
                AbsolutePath::parse(path_value).map_err(|error| DiscoveryError::Invalid {
                    path: path.to_path_buf(),
                    error,
                })?;
            let tags = map_get(map, "tags")
                .map(|value| {
                    let tags = value.as_list().ok_or_else(|| DiscoveryError::Malformed {
                        path: path.to_path_buf(),
                        reason: "repository.tags must be a list".to_owned(),
                    })?;
                    tags.iter()
                        .map(|tag| {
                            let text = tag.as_str().ok_or_else(|| DiscoveryError::Malformed {
                                path: path.to_path_buf(),
                                reason: "repository.tags entries must be strings".to_owned(),
                            })?;
                            RepositoryTag::parse(text).map_err(|error| DiscoveryError::Invalid {
                                path: path.to_path_buf(),
                                error,
                            })
                        })
                        .collect::<Result<Vec<RepositoryTag>, DiscoveryError>>()
                })
                .transpose()?
                .unwrap_or_default();
            DestinationRepository::new(id, repository_path, tags).map_err(|error| {
                DiscoveryError::Invalid {
                    path: path.to_path_buf(),
                    error,
                }
            })
        })
        .collect()
}

fn load_sources(path: &Path, value: &YValue) -> Result<Vec<SourceReference>, DiscoveryError> {
    let items = value.as_list().ok_or_else(|| DiscoveryError::Malformed {
        path: path.to_path_buf(),
        reason: "sources must be a list".to_owned(),
    })?;
    items
        .iter()
        .map(|item| {
            let YValue::Map(map) = item else {
                return Err(DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "source entries must be mappings".to_owned(),
                });
            };
            let id = map_get(map, "id")
                .and_then(YValue::as_str)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "source.id must be a string".to_owned(),
                })
                .and_then(|value| {
                    SourceId::parse(value).map_err(|error| DiscoveryError::Invalid {
                        path: path.to_path_buf(),
                        error,
                    })
                })?;
            let location_text = map_get(map, "location")
                .and_then(YValue::as_str)
                .ok_or_else(|| DiscoveryError::Malformed {
                    path: path.to_path_buf(),
                    reason: "source.location must be a string".to_owned(),
                })?;
            let location = if location_text.starts_with('/') {
                let path_value = AbsolutePath::parse(location_text).map_err(|error| {
                    DiscoveryError::Invalid {
                        path: path.to_path_buf(),
                        error,
                    }
                })?;
                SourceLocation::local(path_value)
            } else {
                SourceLocation::remote(location_text).map_err(|error| DiscoveryError::Invalid {
                    path: path.to_path_buf(),
                    error,
                })?
            };
            Ok(SourceReference::new(id, location))
        })
        .collect()
}

fn agent_kind(name: &str) -> Option<AgentKind> {
    match name {
        "codex" => Some(AgentKind::Codex),
        "claude-code" => Some(AgentKind::ClaudeCode),
        "pi" => Some(AgentKind::Pi),
        _ => None,
    }
}
