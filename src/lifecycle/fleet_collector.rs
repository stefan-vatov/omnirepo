//! Fleet result collection: one initial result per declared member.
//!
//! Every declared fleet member receives exactly one result in declared
//! order; a missing member or a duplicate result fails typed, so no
//! member silently disappears from the accounting.

#![allow(dead_code)]

use crate::lifecycle::fleet_fanout::RepoResult;

#[cfg(test)]
mod fleet_collector_tests;
use std::{error::Error, fmt};

/// One member's collected initial result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemberResult {
    Delivered { repository: String, oid: String },
    Failed { repository: String, reason: String },
    Skipped { repository: String, reason: String },
}

/// The collected fleet results (declared order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetResults {
    pub members: Vec<MemberResult>,
}

/// Collection failures.
#[derive(Debug)]
pub enum CollectError {
    MissingMember { repository: String },
    DuplicateResult { repository: String },
}

impl fmt::Display for CollectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMember { repository } => {
                write!(formatter, "fleet member {repository} has no initial result")
            }
            Self::DuplicateResult { repository } => {
                write!(
                    formatter,
                    "fleet member {repository} has more than one result"
                )
            }
        }
    }
}
impl Error for CollectError {}

/// Collect one result per declared fleet member, in declared order.
pub fn collect_fleet_results(
    results: Vec<RepoResult>,
    declared: &[String],
) -> Result<FleetResults, CollectError> {
    let mut by_repository: Vec<(String, MemberResult)> = Vec::new();
    for result in results {
        let member = match result {
            RepoResult::Delivered { repository, oid } => (
                repository.clone(),
                MemberResult::Delivered { repository, oid },
            ),
            RepoResult::Failed { repository, reason } => (
                repository.clone(),
                MemberResult::Failed { repository, reason },
            ),
            RepoResult::Skipped { repository, reason } => (
                repository.clone(),
                MemberResult::Skipped { repository, reason },
            ),
        };
        if by_repository.iter().any(|(id, _)| *id == member.0) {
            return Err(CollectError::DuplicateResult {
                repository: member.0,
            });
        }
        by_repository.push(member);
    }
    let mut members = Vec::with_capacity(declared.len());
    for repository in declared {
        let Some((_, member)) = by_repository.iter().find(|(id, _)| id == repository) else {
            return Err(CollectError::MissingMember {
                repository: repository.clone(),
            });
        };
        members.push(member.clone());
    }
    Ok(FleetResults { members })
}
