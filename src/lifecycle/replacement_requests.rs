//! Whole-file plan operation mapping into exact replacement requests.
//!
//! Every whole-file selected plan operation yields one contained
//! replacement request; section items are rejected here (the whole-file
//! leaf owns this mapping).  Source, configuration, and plan identities
//! are preserved on the request for journaling and revalidation.

#![allow(dead_code)]

use super::sync_plan::{PlanDecision, SyncPlan};
use crate::platform::RelativePath;
use crate::source::ItemKind;
use std::{error::Error, fmt};

/// One exact whole-file replacement request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacementRequest {
    pub plan_item_id: String,
    pub target: RelativePath,
    pub source_identity: String,
    pub configuration_identity: String,
    pub plan_identity: String,
}

/// Mapping failures.
#[derive(Debug)]
pub enum RequestError {
    SectionItem { id: String },
    InvalidTarget { id: String, reason: String },
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SectionItem { id } => {
                write!(
                    formatter,
                    "plan item {id} is a section; this leaf maps whole files only"
                )
            }
            Self::InvalidTarget { id, reason } => {
                write!(formatter, "plan item {id} has an invalid target: {reason}")
            }
        }
    }
}
impl Error for RequestError {}

/// Map every whole-file selected plan operation into one contained
/// replacement request.  Section items fail typed; rejected items are
/// skipped (they already carry their reason).
pub fn map_whole_file_requests(
    plan: &SyncPlan,
    source_identity: &str,
    configuration_identity: &str,
) -> Result<Vec<ReplacementRequest>, RequestError> {
    let plan_identity = plan.render();
    let mut requests = Vec::new();
    for item in &plan.items {
        if !matches!(item.decision, PlanDecision::Selected { .. }) {
            continue;
        }
        if item.kind != ItemKind::WholeFile {
            return Err(RequestError::SectionItem {
                id: item.id.clone(),
            });
        }
        let relative =
            RelativePath::parse(&item.target).map_err(|error| RequestError::InvalidTarget {
                id: item.id.clone(),
                reason: error.to_string(),
            })?;
        requests.push(ReplacementRequest {
            plan_item_id: item.id.clone(),
            target: relative,
            source_identity: source_identity.to_owned(),
            configuration_identity: configuration_identity.to_owned(),
            plan_identity: plan_identity.clone(),
        });
    }
    Ok(requests)
}

#[cfg(test)]
mod replacement_requests_tests;
