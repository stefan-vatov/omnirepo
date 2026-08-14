//! Internal initial fleet application request and response.
//!
//! The request is internal-only: it accepts no URL, local file, template,
//! or destination override — the fleet, machine identity, and bounded
//! limit come from the canonical configuration.  The response carries
//! every initial result and the frozen repair inputs.  Adapters are
//! injected by the caller; rendering and agents are absent.

#![allow(dead_code)]

use crate::lifecycle::fleet_collector::MemberResult;
use crate::lifecycle::plan_selection::Policy;
use crate::lifecycle::repository_preflight::{RepoPreflight, preflight_repositories};
use crate::lifecycle::sync_plan::PlanItem;
use crate::lifecycle::work_mapping::{WorkItem, map_preflight_to_work};
use crate::source::{CatalogState, SourceCatalog, SourceId};

#[cfg(test)]
mod fleet_app_tests;

#[cfg(test)]
mod fleet_composition_tests;
use std::{error::Error, fmt};

/// The internal application request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetRequest {
    pub fleet: Vec<String>,
    pub machine_identity: String,
    pub limit: usize,
}

/// Request failures.
#[derive(Debug)]
pub enum RequestError {
    EmptyFleet,
    ZeroLimit,
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFleet => write!(formatter, "the fleet is empty"),
            Self::ZeroLimit => write!(formatter, "the bounded limit is zero"),
        }
    }
}
impl Error for RequestError {}

/// Build the internal request from the canonical inputs.
pub fn build_request(
    fleet: &[String],
    machine_identity: &str,
    limit: usize,
) -> Result<FleetRequest, RequestError> {
    if fleet.is_empty() {
        return Err(RequestError::EmptyFleet);
    }
    if limit == 0 {
        return Err(RequestError::ZeroLimit);
    }
    Ok(FleetRequest {
        fleet: fleet.to_vec(),
        machine_identity: machine_identity.to_owned(),
        limit,
    })
}

/// The internal application response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetResponse {
    pub run_id: String,
    pub results: Vec<MemberResult>,
    /// Frozen repair inputs: (repository, frozen repair candidate).
    pub frozen_repair_inputs: Vec<(String, String)>,
}

/// Response failures.
#[derive(Debug)]
pub enum ResponseError {
    EmptyResults,
    RepairInputWithoutFailure { repository: String },
}

impl fmt::Display for ResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyResults => write!(formatter, "the response has no results"),
            Self::RepairInputWithoutFailure { repository } => {
                write!(formatter, "repair input {repository} has no failed member")
            }
        }
    }
}
impl Error for ResponseError {}

/// The composed fleet: the internal request, the scheduler work, the
/// admitted repositories, and the per-repository accounting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetComposition {
    pub request: FleetRequest,
    pub work: Vec<WorkItem>,
    /// Repositories admitted to the initial accounting.
    pub accounted: Vec<String>,
    /// Repositories skipped by preflight with their reasons.
    pub skipped: Vec<(String, String)>,
    /// Repositories affected by invalid machine or source state.
    pub affected: Vec<String>,
}

impl WorkItem {
    pub(crate) fn is_run(&self) -> bool {
        matches!(self, WorkItem::Run { .. })
    }

    pub(crate) fn is_skip(&self) -> bool {
        matches!(self, WorkItem::Skip { .. })
    }
}

/// Compose the fleet application from the canonical inputs.  The shared
/// preflight (machine validity + source catalog) gates everything; the
/// per-repository policy/plan preflight decides admission; every
/// determinable repository enters the initial accounting.  No
/// source/cache/lease/destination effect happens here.
pub fn compose_fleet(
    machine_valid: bool,
    catalog: &SourceCatalog,
    repositories: &[(&str, Option<&Policy>, &[PlanItem])],
    limit: usize,
) -> Result<FleetComposition, RequestError> {
    let mut affected = Vec::new();
    if repositories.is_empty() {
        return Ok(FleetComposition {
            request: FleetRequest {
                fleet: Vec::new(),
                machine_identity: String::new(),
                limit,
            },
            work: Vec::new(),
            accounted: Vec::new(),
            skipped: Vec::new(),
            affected,
        });
    }
    if !machine_valid {
        for (repository, _, _) in repositories {
            affected.push((*repository).to_owned());
        }
        return Ok(FleetComposition {
            request: FleetRequest {
                fleet: Vec::new(),
                machine_identity: String::new(),
                limit,
            },
            work: Vec::new(),
            accounted: Vec::new(),
            skipped: Vec::new(),
            affected,
        });
    }
    // Source availability gates the declarations: an unavailable or
    // shadowed source affects its repositories.
    for (repository, _, items) in repositories {
        for item in *items {
            let state = catalog
                .entries()
                .iter()
                .find(|entry| catalog_source(entry).is_some_and(|id| id.as_str() == item.source));
            match state {
                Some(CatalogState::Complete { .. }) => {}
                _ => {
                    affected.push((*repository).to_owned());
                    break;
                }
            }
        }
    }
    let preflight = preflight_repositories(repositories);
    let mut admitted = Vec::new();
    let mut skipped = Vec::new();
    let mut work_items = Vec::new();
    for outcome in &preflight {
        match outcome {
            RepoPreflight::ReadyPlan { repository } => {
                if affected.iter().any(|id| id == repository) {
                    work_items.push(WorkItem::Skip {
                        repository: repository.clone(),
                        reason: "affected by invalid source state".to_owned(),
                    });
                } else {
                    admitted.push(repository.clone());
                    work_items.push(WorkItem::Run {
                        repository: repository.clone(),
                        plan_identity: "plan".to_owned(),
                    });
                }
            }
            RepoPreflight::Failed {
                repository,
                reasons,
            } => {
                skipped.push((repository.clone(), reasons.join("; ")));
                work_items.push(WorkItem::Skip {
                    repository: repository.clone(),
                    reason: reasons.join("; "),
                });
            }
        }
    }
    let work = if work_items.is_empty() {
        map_preflight_to_work(&preflight)
    } else {
        work_items
    };
    let request = build_request(&admitted, "machine-1", limit)?;
    Ok(FleetComposition {
        request,
        work,
        accounted: admitted,
        skipped,
        affected,
    })
}

fn catalog_source(state: &CatalogState) -> Option<&SourceId> {
    match state {
        CatalogState::Complete { source, .. }
        | CatalogState::Shadowed { source, .. }
        | CatalogState::Unavailable { source, .. } => Some(source),
    }
}

/// Finalize the response: every initial result and the frozen repair
/// inputs, checked for consistency.
pub fn finalize_response(
    run_id: &str,
    results: Vec<MemberResult>,
    frozen_repair_inputs: Vec<(String, String)>,
) -> Result<FleetResponse, ResponseError> {
    if results.is_empty() {
        return Err(ResponseError::EmptyResults);
    }
    for (repository, _) in &frozen_repair_inputs {
        let failed = results
            .iter()
            .any(|member| matches!(member, MemberResult::Failed { repository: id, .. } if id == repository));
        if !failed {
            return Err(ResponseError::RepairInputWithoutFailure {
                repository: repository.clone(),
            });
        }
    }
    Ok(FleetResponse {
        run_id: run_id.to_owned(),
        results,
        frozen_repair_inputs,
    })
}
