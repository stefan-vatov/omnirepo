//! Freeze machine concurrency and compose the fleet.
//!
//! The machine concurrency limits freeze for the run: the configured
//! cap or the fixed default (max_repositories 4, max_child_work 8);
//! transient sync overrides are accepted only when they lower the
//! machine cap and never persist or raise it.  The fleet composes from
//! the per-repository plans in declared order; a failed plan affects
//! only its repository while its peers keep their work.  Completion
//! order never alters authority or accounting.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_composition_tests;

use crate::configuration::{MachineConcurrency, MachineConfiguration};
use crate::lifecycle::fleet_app::{FleetComposition, compose_fleet};
use crate::lifecycle::fleet_planning::RepositoryPlan;
use crate::lifecycle::plan_selection::Policy;
use crate::source::SourceCatalog;

/// The frozen default for `max_repositories` when the machine mapping
/// omits it.
pub const DEFAULT_MAX_REPOSITORIES: u16 = 4;
/// The frozen default for `max_child_work` when the machine mapping
/// omits it.
pub const DEFAULT_MAX_CHILD_WORK: u16 = 8;

/// The composition outcome: the fleet plus the frozen limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionOutcome {
    pub composition: FleetComposition,
    pub limit: usize,
}

/// Freeze the repository concurrency limit for the run.
///
/// The machine cap (or its fixed default) governs; a transient override
/// is accepted only when it lowers the cap — an equal or higher override
/// is an invocation error and never raises the machine authority.
pub fn freeze_concurrency(
    machine: MachineConcurrency,
    override_max: Option<u16>,
) -> Result<u16, String> {
    let cap = machine.max_repositories();
    match override_max {
        None => Ok(cap),
        Some(requested) if requested <= cap => Ok(requested),
        Some(requested) => Err(format!(
            "the sync override {requested} exceeds the machine cap {cap}; overrides only lower"
        )),
    }
}

/// Compose the fleet from the machine authority, the catalog, and the
/// per-repository plans.
///
/// A repository whose plan failed is affected (with its reason) while it
/// stays in the accounting; its peers keep their work.  The fleet and
/// the work preserve declared order.
pub fn compose_configured_fleet(
    config: &MachineConfiguration,
    catalog: &SourceCatalog,
    plans: &[RepositoryPlan],
    override_max: Option<u16>,
) -> Result<CompositionOutcome, String> {
    let limit = freeze_concurrency(config.concurrency(), override_max)? as usize;
    let mut affected = Vec::new();
    let mut entries = Vec::new();
    for plan in plans {
        // A failed plan affects only its repository: it never enters the
        // normal composition path (an empty-items repo would silently
        // succeed), and it stays out of the work while the accounting
        // keeps it via the affected list.
        let policy: Option<Policy> = match &plan.plan {
            Ok(_) => Some(Policy::Absent),
            Err(reason) => {
                affected.push(format!("{}: {reason}", plan.repository));
                continue;
            }
        };
        // Only selected items compose work: rejected losers never
        // execute, so they must not gate admission or availability.
        let items: Vec<crate::lifecycle::sync_plan::PlanItem> = match &plan.plan {
            Ok(plan) => plan
                .items
                .iter()
                .filter(|item| {
                    matches!(
                        item.decision,
                        crate::lifecycle::sync_plan::PlanDecision::Selected { .. }
                    )
                })
                .cloned()
                .collect(),
            Err(_) => Vec::new(),
        };
        entries.push((plan.repository.clone(), policy, items));
    }
    let repositories = entries
        .iter()
        .map(|(id, policy, items)| (id.as_str(), policy.as_ref(), items.as_slice()))
        .collect::<Vec<_>>();
    let mut composition =
        compose_fleet(true, catalog, &repositories, limit).map_err(|error| error.to_string())?;
    for entry in affected {
        if !composition.affected.contains(&entry) {
            composition.affected.push(entry);
        }
    }
    Ok(CompositionOutcome { composition, limit })
}
