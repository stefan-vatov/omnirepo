//! Shared machine and policy-scoped source preflight.
//!
//! Invalid machine state admits no source or destination effect: every
//! source is diagnosed and nothing is eligible.  Source diagnostics name
//! the precedence position and the affected plans.  Under a valid policy,
//! repositories that depend only on complete sources remain eligible.

#![allow(dead_code)]

use crate::source::{CatalogState, SourceCatalog, SourceId};

/// One source's preflight diagnosis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDiagnostic {
    pub source: SourceId,
    /// The precedence position (1-based declared order).
    pub position: usize,
    pub state: SourceState,
    /// The plans (repositories) this source affects.
    pub affected_plans: Vec<String>,
}

/// The diagnosed source state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceState {
    Complete,
    Unavailable { reason: String },
    Shadowed { by: SourceId },
}

/// The preflight report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreflightReport {
    pub machine_valid: bool,
    pub sources: Vec<SourceDiagnostic>,
    /// Repositories eligible under the policy.
    pub eligible: Vec<String>,
    /// Repositories affected by an invalid machine or source state.
    pub affected: Vec<String>,
}

/// Run the shared preflight.  When the machine state is invalid, no source
/// or destination effect is admitted.
pub fn preflight(
    catalog: &SourceCatalog,
    machine_valid: bool,
    repositories: &[(&str, &SourceId)],
) -> PreflightReport {
    let mut sources = Vec::new();
    for (position, entry) in catalog.entries().iter().enumerate() {
        let (source, state) = match entry {
            CatalogState::Complete { source, .. } => (source.clone(), SourceState::Complete),
            CatalogState::Unavailable { source, reason } => (
                source.clone(),
                SourceState::Unavailable {
                    reason: reason.clone(),
                },
            ),
            CatalogState::Shadowed { source, by } => {
                (source.clone(), SourceState::Shadowed { by: by.clone() })
            }
        };
        let affected_plans = repositories
            .iter()
            .filter(|(_, declared)| *declared == &source)
            .map(|(repository, _)| (*repository).to_owned())
            .collect();
        sources.push(SourceDiagnostic {
            source,
            position: position + 1,
            state,
            affected_plans,
        });
    }

    let mut eligible = Vec::new();
    let mut affected = Vec::new();
    for (repository, declared) in repositories {
        let diagnosed = sources.iter().find(|entry| &entry.source == *declared);
        let ok =
            machine_valid && diagnosed.is_some_and(|entry| entry.state == SourceState::Complete);
        if ok {
            eligible.push((*repository).to_owned());
        } else {
            affected.push((*repository).to_owned());
        }
    }
    PreflightReport {
        machine_valid,
        sources,
        eligible,
        affected,
    }
}

#[cfg(test)]
mod preflight_tests;
