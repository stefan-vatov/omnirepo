//! Bind source declarations to configured repositories by
//! applicability.
//!
//! Each parsed declaration maps to an ItemDeclaration for every
//! destination whose stable machine-declared tags match the
//! declaration's applicability tags; an untagged declaration applies to
//! every destination.  The binding is pure: it uses only the machine
//! configuration and the declarations and never probes destination
//! content.  Declared order is preserved.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_binding_tests;

use crate::configuration::MachineConfiguration;
use crate::source::{ItemDeclaration, ItemKind, SourceDeclaration};
use std::error::Error;
use std::fmt;

/// Binding failures.
#[derive(Debug)]
pub enum BindingError {
    MissingId {
        source: String,
        provenance: String,
    },
    MissingDestination {
        source: String,
        provenance: String,
    },
    InvalidMode {
        source: String,
        provenance: String,
        mode: String,
    },
    InvalidId {
        source: String,
        provenance: String,
        id: String,
    },
    ProtectedTarget {
        source: String,
        provenance: String,
        target: String,
    },
    MissingSectionId {
        source: String,
        provenance: String,
    },
    InvalidSectionId {
        source: String,
        provenance: String,
        section: String,
    },
    SectionOnWholeFile {
        source: String,
        provenance: String,
        section: String,
    },
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingId { source, provenance } => {
                write!(
                    formatter,
                    "source {source} declaration {provenance} has no id"
                )
            }
            Self::MissingDestination { source, provenance } => write!(
                formatter,
                "source {source} declaration {provenance} has no destination path"
            ),
            Self::InvalidMode {
                source,
                provenance,
                mode,
            } => write!(
                formatter,
                "source {source} declaration {provenance} has unknown mode {mode:?}"
            ),
            Self::InvalidId {
                source,
                provenance,
                id,
            } => write!(
                formatter,
                "source {source} declaration {provenance} has an invalid id {id:?}: ids use ASCII lowercase letters, digits, dots, underscores, and hyphens"
            ),
            Self::ProtectedTarget {
                source,
                provenance,
                target,
            } => write!(
                formatter,
                "source {source} declaration {provenance} targets the protected authority file {target:?}"
            ),
            Self::MissingSectionId { source, provenance } => write!(
                formatter,
                "source {source} declaration {provenance} has mode=section but no section id"
            ),
            Self::InvalidSectionId {
                source,
                provenance,
                section,
            } => write!(
                formatter,
                "source {source} declaration {provenance} has an invalid section id {section:?}"
            ),
            Self::SectionOnWholeFile {
                source,
                provenance,
                section,
            } => write!(
                formatter,
                "source {source} declaration {provenance} declares section {section:?} without mode=section"
            ),
        }
    }
}
impl Error for BindingError {}

/// Bind the declarations to every applicable destination, in declared
/// source order.  Returns one entry per configured destination.
pub fn bind_declarations(
    config: &MachineConfiguration,
    declarations: &[SourceDeclaration],
) -> Result<Vec<(String, Vec<ItemDeclaration>)>, BindingError> {
    let mut bindings = Vec::new();
    for destination in config.repositories() {
        let repository = destination.id().as_str().to_owned();
        let destination_tags = destination
            .tags()
            .iter()
            .map(|tag| tag.as_str().to_owned())
            .collect::<Vec<_>>();
        let mut items = Vec::new();
        for (index, declaration) in declarations.iter().enumerate() {
            if !applies_to(declaration, &destination_tags) {
                continue;
            }
            items.push(to_item(declaration, index)?);
        }
        bindings.push((repository, items));
    }
    Ok(bindings)
}

/// Applicability: the declaration's `tags` field lists comma-separated
/// stable tags; an empty list applies to every destination, otherwise at
/// least one tag must match a machine-declared destination tag.
fn applies_to(declaration: &SourceDeclaration, destination_tags: &[String]) -> bool {
    let tags = field(declaration, "tags");
    let declared = tags
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    declared.is_empty()
        || declared
            .iter()
            .any(|tag| destination_tags.iter().any(|t| t == tag))
}

/// Convert one parsed declaration to the item declaration.
fn to_item(declaration: &SourceDeclaration, index: usize) -> Result<ItemDeclaration, BindingError> {
    let id = field(declaration, "id").to_owned();
    if id.is_empty() {
        return Err(BindingError::MissingId {
            source: declaration.source.as_str().to_owned(),
            provenance: declaration.provenance.clone(),
        });
    }
    // Item IDs follow the same stable slug rule as section IDs
    // (canon/architecture/managed-content.md).
    if !crate::configuration::is_valid_section_id(&id) {
        return Err(BindingError::InvalidId {
            source: declaration.source.as_str().to_owned(),
            provenance: declaration.provenance.clone(),
            id,
        });
    }
    let target = field(declaration, "destination").to_owned();
    if target.is_empty() {
        return Err(BindingError::MissingDestination {
            source: declaration.source.as_str().to_owned(),
            provenance: declaration.provenance.clone(),
        });
    }
    // Protected authority-file targets fail before destination mutation
    // (canon/architecture/managed-content.md): a source may never manage
    // a destination's own configuration authority.
    if target == ".omnirepo.yaml" || target == ".omnirepo" || target.starts_with(".omnirepo/") {
        return Err(BindingError::ProtectedTarget {
            source: declaration.source.as_str().to_owned(),
            provenance: declaration.provenance.clone(),
            target,
        });
    }
    let section_field = field(declaration, "section");
    let (kind, section) = match field(declaration, "mode") {
        "sync" | "whole" | "" => {
            if !section_field.is_empty() {
                return Err(BindingError::SectionOnWholeFile {
                    source: declaration.source.as_str().to_owned(),
                    provenance: declaration.provenance.clone(),
                    section: section_field.to_owned(),
                });
            }
            (ItemKind::WholeFile, None)
        }
        "section" => {
            if section_field.is_empty() {
                return Err(BindingError::MissingSectionId {
                    source: declaration.source.as_str().to_owned(),
                    provenance: declaration.provenance.clone(),
                });
            }
            let section = crate::configuration::SectionId::new(section_field).map_err(|_| {
                BindingError::InvalidSectionId {
                    source: declaration.source.as_str().to_owned(),
                    provenance: declaration.provenance.clone(),
                    section: section_field.to_owned(),
                }
            })?;
            (ItemKind::Section, Some(section))
        }
        other => {
            return Err(BindingError::InvalidMode {
                source: declaration.source.as_str().to_owned(),
                provenance: declaration.provenance.clone(),
                mode: other.to_owned(),
            });
        }
    };
    Ok(ItemDeclaration {
        id,
        target,
        source: declaration.source.as_str().to_owned(),
        source_path: declaration.path.clone(),
        kind,
        section,
        source_order: index,
    })
}

fn field<'a>(declaration: &'a SourceDeclaration, key: &str) -> &'a str {
    declaration
        .fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}
