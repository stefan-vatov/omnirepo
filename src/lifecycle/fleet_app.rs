//! Internal initial fleet application request and response.
//!
//! The request is internal-only: it accepts no URL, local file, template,
//! or destination override — the fleet, machine identity, and bounded
//! limit come from the canonical configuration.  The response carries
//! every initial result and the frozen repair inputs.  Adapters are
//! injected by the caller; rendering and agents are absent.

#![allow(dead_code)]

use crate::lifecycle::fleet_collector::MemberResult;

#[cfg(test)]
mod fleet_app_tests;
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
