//! Source catalog states and planning impact.
//!
//! The catalog distinguishes complete, shadowed, and unavailable source
//! authority.  Planning explains which repositories are affected by an
//! unavailable source (their authoritative plan cannot be proven) and
//! which peers remain independent and eligible.  An unavailable source is
//! never silently removed, reordered, or replaced by a lower-precedence
//! source; invalid machine authority is global and makes every source
//! unavailable.

#![allow(dead_code)]

use super::snapshot::{RevisionId, SourceId};
use std::{error::Error, fmt};

/// One source's catalog state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogState {
    /// The snapshot and its declarations are complete.
    Complete {
        source: SourceId,
        revision: RevisionId,
    },
    /// A higher-precedence source covers this source (declared order).
    Shadowed { source: SourceId, by: SourceId },
    /// The source could not be acquired or declared; typed reason.
    Unavailable { source: SourceId, reason: String },
}

/// Catalog failures.
#[derive(Debug)]
pub enum CatalogError {
    /// Invalid machine authority is global: every source is unavailable.
    GlobalAuthority {
        reason: String,
    },
    Duplicate {
        source: SourceId,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GlobalAuthority { reason } => {
                write!(formatter, "invalid machine authority is global: {reason}")
            }
            Self::Duplicate { source } => {
                write!(
                    formatter,
                    "source {} is declared more than once",
                    source.as_str()
                )
            }
        }
    }
}
impl Error for CatalogError {}

/// The ordered source catalog (declared precedence).
#[derive(Clone, Debug, Default)]
pub struct SourceCatalog {
    entries: Vec<CatalogState>,
}

impl SourceCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one source in declared order.  A source may be entered only
    /// once.
    pub fn record(&mut self, state: CatalogState) -> Result<(), CatalogError> {
        let source = match &state {
            CatalogState::Complete { source, .. }
            | CatalogState::Shadowed { source, .. }
            | CatalogState::Unavailable { source, .. } => source.clone(),
        };
        if self
            .entries
            .iter()
            .any(|entry| catalog_source(entry) == Some(&source))
        {
            return Err(CatalogError::Duplicate { source });
        }
        self.entries.push(state);
        Ok(())
    }

    /// The declared entries in order.
    pub fn entries(&self) -> &[CatalogState] {
        &self.entries
    }
}

fn catalog_source(state: &CatalogState) -> Option<&SourceId> {
    match state {
        CatalogState::Complete { source, .. }
        | CatalogState::Shadowed { source, .. }
        | CatalogState::Unavailable { source, .. } => Some(source),
    }
}

/// Planning impact of one repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanningImpact {
    /// The repository's authoritative plan cannot be proven.
    Affected { repository: String, reason: String },
    /// The repository is provably independent of every unavailable source.
    Independent { repository: String },
}

/// Explain the planning impact: a repository is affected when its declared
/// source is unavailable or shadowed; peers whose source is complete remain
/// independent and eligible.  Unavailable sources never promote a
/// lower-precedence replacement.
pub fn plan_impact(
    catalog: &SourceCatalog,
    repository_sources: &[(&str, &SourceId)],
) -> Vec<PlanningImpact> {
    let mut impacts = Vec::new();
    for (repository, source) in repository_sources {
        let state = catalog
            .entries()
            .iter()
            .find(|entry| catalog_source(entry) == Some(*source));
        let impact = match state {
            Some(CatalogState::Complete { .. }) => PlanningImpact::Independent {
                repository: (*repository).to_owned(),
            },
            Some(CatalogState::Shadowed { by, .. }) => PlanningImpact::Affected {
                repository: (*repository).to_owned(),
                reason: format!(
                    "source {} is shadowed by the higher-precedence source {}",
                    source.as_str(),
                    by.as_str()
                ),
            },
            Some(CatalogState::Unavailable { reason, .. }) => PlanningImpact::Affected {
                repository: (*repository).to_owned(),
                reason: format!("source {} is unavailable: {reason}", source.as_str()),
            },
            None => PlanningImpact::Affected {
                repository: (*repository).to_owned(),
                reason: format!("source {} is not declared", source.as_str()),
            },
        };
        impacts.push(impact);
    }
    impacts
}
