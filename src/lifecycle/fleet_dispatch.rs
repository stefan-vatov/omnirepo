//! The dispatch seam: machine config to fleet run to exit.
//!
//! `dispatch_fleet` composes the canonical pipeline for a configured
//! machine authority: source catalog, pinned declarations, per-repository
//! policies and plans, the composed fleet, the bounded initial passes,
//! the bounded repair pass, and the finalized summary with the exact
//! exit class.  An absent machine authority is the empty-fleet success.
//! Discovery checks only `<HOME>/.omnirepo/config.yaml`; the CLI cannot
//! substitute another machine authority file.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_dispatch_tests;

use crate::configuration::{Discovery, discover};
use crate::lifecycle::adapters::resolve_adapters;
use crate::lifecycle::exit_status::ExitClass;
use crate::lifecycle::fleet_binding::bind_declarations;
use crate::lifecycle::fleet_catalog::{
    build_runtime_catalog, build_sync_runtime_catalog, materialized_source_roots,
};
use crate::lifecycle::fleet_composition::compose_configured_fleet;
use crate::lifecycle::fleet_declarations::read_pinned_declarations;
use crate::lifecycle::fleet_finalize::finalize_fleet_run;
use crate::lifecycle::fleet_planning::build_repository_plans;
use crate::lifecycle::fleet_policy::load_repository_policies;
use crate::lifecycle::fleet_repair::{FailedMember, run_fleet_repair};
use crate::lifecycle::fleet_runner::run_fleet_initial_passes;
use crate::lifecycle::journal::JournalHandle;
use crate::source::{CatalogState, RevisionId};
use std::path::Path;

/// The dispatch outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchOutcome {
    pub exit_class: ExitClass,
    pub repositories: usize,
}

/// Dispatch failures (the typed fail-closed seam).
#[derive(Debug)]
pub enum DispatchError {
    Discovery { reason: String },
    Pipeline { reason: String },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Discovery { reason } => write!(formatter, "dispatch discovery failure: {reason}"),
            Self::Pipeline { reason } => write!(formatter, "dispatch pipeline failure: {reason}"),
        }
    }
}
impl std::error::Error for DispatchError {}

/// Dispatch the canonical fleet application for the machine authority.
pub fn dispatch_fleet(
    journal: &JournalHandle,
    run_id: &str,
    home: &Path,
    record_path: &Path,
) -> Result<DispatchOutcome, DispatchError> {
    let discovery = discover(home).map_err(|error| DispatchError::Discovery {
        reason: error.to_string(),
    })?;
    match discovery {
        // Absent machine authority: the empty-fleet success contract.
        Discovery::Absent => {
            journal
                .submit(crate::lifecycle::event::JournalEvent::Terminal {
                    checkpoint: 0,
                    run_id: run_id.to_owned(),
                    outcome: crate::lifecycle::event::Outcome::Success,
                })
                .map_err(|error| DispatchError::Pipeline {
                    reason: error.to_string(),
                })?;
            Ok(DispatchOutcome {
                exit_class: ExitClass::Success,
                repositories: 0,
            })
        }
        Discovery::Present(config) => run_configured(journal, run_id, &config, record_path),
    }
}

/// The effect-free planning prefix shared by `sync` and `doctor`:
/// catalog, pinned declarations, policies, bindings, and the
/// per-repository plans.  No destination managed content is read or
/// written; only each destination's `.omnirepo.yaml` policy is loaded.
pub struct FleetPlanning {
    pub catalog: crate::source::SourceCatalog,
    pub plans: Vec<crate::lifecycle::fleet_planning::RepositoryPlan>,
    pub source_roots: std::collections::HashMap<String, std::path::PathBuf>,
}

/// Build the planning prefix for a configured machine authority.
pub fn plan_configured_fleet(
    config: &crate::configuration::MachineConfiguration,
) -> Result<FleetPlanning, DispatchError> {
    let catalog = build_runtime_catalog(config).map_err(|error| DispatchError::Pipeline {
        reason: error.to_string(),
    })?;
    plan_with_catalog(config, catalog)
}

fn plan_configured_sync(
    config: &crate::configuration::MachineConfiguration,
) -> Result<FleetPlanning, DispatchError> {
    let catalog = build_sync_runtime_catalog(config).map_err(|error| DispatchError::Pipeline {
        reason: error.to_string(),
    })?;
    plan_with_catalog(config, catalog)
}

fn plan_with_catalog(
    config: &crate::configuration::MachineConfiguration,
    catalog: crate::source::SourceCatalog,
) -> Result<FleetPlanning, DispatchError> {
    let source_roots =
        materialized_source_roots(config, &catalog).map_err(|error| DispatchError::Pipeline {
            reason: error.to_string(),
        })?;
    let mut declarations = Vec::new();
    for state in catalog.entries() {
        if let CatalogState::Complete { source, revision } = state {
            let source_root = source_roots.get(source.as_str());
            if let Some(source_root) = source_root {
                let pinned_revision = RevisionId::new(revision.as_str()).map_err(|error| {
                    DispatchError::Pipeline {
                        reason: error.to_string(),
                    }
                })?;
                let parsed = read_pinned_declarations(source, &pinned_revision, source_root)
                    .map_err(|error| DispatchError::Pipeline { reason: error })?;
                declarations.extend(parsed);
            }
        }
    }
    // 3. The per-destination policies (lawful absence).
    let policies = load_repository_policies(config);
    // 4. The declaration bindings by applicability.
    let bindings =
        bind_declarations(config, &declarations).map_err(|error| DispatchError::Pipeline {
            reason: error.to_string(),
        })?;
    // 5. The per-repository plans.
    let plans = build_repository_plans(config, &catalog, &bindings, &policies);
    Ok(FleetPlanning {
        catalog,
        plans,
        source_roots,
    })
}

/// Run the configured fleet pipeline end to end.
fn run_configured(
    journal: &JournalHandle,
    run_id: &str,
    config: &crate::configuration::MachineConfiguration,
    record_path: &Path,
) -> Result<DispatchOutcome, DispatchError> {
    // Materialize remote sources for this sync, then run the same planning
    // logic that doctor uses for already materialized snapshots.
    let FleetPlanning {
        catalog,
        plans,
        source_roots,
    } = plan_configured_sync(config)?;
    // 6. The composed fleet with the frozen concurrency limit.
    let composed = compose_configured_fleet(config, &catalog, &plans, None).map_err(|error| {
        DispatchError::Pipeline {
            reason: error.to_string(),
        }
    })?;
    // 7. The bounded initial passes.
    let response = run_fleet_initial_passes(
        journal,
        run_id,
        config,
        &plans,
        &source_roots,
        &composed.composition,
        composed.limit,
    )
    .map_err(|error| DispatchError::Pipeline { reason: error })?;
    // 8. The bounded repair pass over the failed members.
    let failed = response
        .results
        .iter()
        .filter_map(|result| match result {
            crate::lifecycle::fleet_collector::MemberResult::Failed { repository, reason } => {
                Some(FailedMember {
                    repository: repository.clone(),
                    reason: reason.clone(),
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let adapters = match resolve_adapters(config.repair().priority(), &[]) {
        Ok(crate::lifecycle::adapters::AdapterOutcome::Resolved(adapters)) => adapters,
        _ => Vec::new(),
    };
    let record_text = std::fs::read_to_string(record_path).unwrap_or_default();
    let repair = run_fleet_repair(journal, run_id, config, &failed, &adapters, &record_text)
        .map_err(|error| DispatchError::Pipeline { reason: error })?;
    // The repaired members fold back into the response as delivered.
    let mut final_results = response.results.clone();
    for repository in &repair.repaired {
        for result in &mut final_results {
            if let crate::lifecycle::fleet_collector::MemberResult::Failed {
                repository: name,
                ..
            } = result
            {
                if name == repository {
                    *result = crate::lifecycle::fleet_collector::MemberResult::Delivered {
                        repository: repository.clone(),
                        oid: "repair".to_owned(),
                    };
                }
            }
        }
    }
    let repaired_response = crate::lifecycle::fleet_app::FleetResponse {
        run_id: response.run_id.clone(),
        results: final_results,
        frozen_repair_inputs: response.frozen_repair_inputs.clone(),
    };
    // 9. Finalize with the post-repair outcomes.
    let finalize = finalize_fleet_run(
        journal,
        run_id,
        &repaired_response,
        &composed.composition.affected,
    )
    .map_err(|error| DispatchError::Pipeline { reason: error })?;
    let repositories = finalize.summary.repositories.len();
    Ok(DispatchOutcome {
        exit_class: finalize.exit_class,
        repositories,
    })
}
