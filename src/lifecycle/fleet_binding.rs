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
    let target = field(declaration, "destination").to_owned();
    if target.is_empty() {
        return Err(BindingError::MissingDestination {
            source: declaration.source.as_str().to_owned(),
            provenance: declaration.provenance.clone(),
        });
    }
    let kind = match field(declaration, "mode") {
        "sync" | "whole" | "" => ItemKind::WholeFile,
        "section" => ItemKind::Section,
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
        kind,
        section: None,
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
