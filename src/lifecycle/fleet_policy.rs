//! Per-destination repository policy loading with lawful absence.
//!
//! Each configured destination loads its exact `.omnirepo.yaml` through
//! the typed loader: a policy failure (malformed, aliased, competing,
//! legacy, or unreadable) fails that destination only and never stops
//! its peers.  Absence is lawful — the plan policy is Absent and
//! inference governs.  A present policy converts to the plan policy
//! exactly: omitted selectors select nothing and never infer.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_policy_tests;

use crate::configuration::MachineConfiguration;
use crate::lifecycle::plan_selection::Policy;
use crate::repository::{PolicyPresence, VerificationCommand, load_policy};
use std::path::Path;

/// One destination's policy load outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPolicyLoad {
    pub repository: String,
    /// The plan policy: None when the load failed, Absent when no policy
    /// file exists, Explicit otherwise.
    pub policy: Option<Policy>,
    /// The declared verification commands (explicit argv arrays) for the
    /// fleet pass; empty when the policy is absent, failed, or declares
    /// no commands.
    pub checks: Vec<VerificationCommand>,
    /// The typed failure for this destination only.
    pub failure: Option<String>,
}

/// Load every configured destination's policy in declared order.  A
/// failure isolates its destination; peers continue.
pub fn load_repository_policies(config: &MachineConfiguration) -> Vec<RepositoryPolicyLoad> {
    config
        .repositories()
        .iter()
        .map(|destination| {
            let repository = destination.id().as_str().to_owned();
            let path = Path::new(destination.path().as_str());
            match load_policy(path) {
                Ok(PolicyPresence::Absent) => RepositoryPolicyLoad {
                    repository,
                    policy: Some(Policy::Absent),
                    checks: Vec::new(),
                    failure: None,
                },
                Ok(PolicyPresence::Present(policy)) => RepositoryPolicyLoad {
                    repository,
                    checks: policy
                        .commands()
                        .as_slice()
                        .map(|commands| commands.to_vec())
                        .unwrap_or_default(),
                    policy: Some(convert(&policy)),
                    failure: None,
                },
                Err(error) => RepositoryPolicyLoad {
                    repository,
                    policy: None,
                    checks: Vec::new(),
                    failure: Some(error.to_string()),
                },
            }
        })
        .collect()
}

/// Convert the validated repository policy to the plan policy exactly.
///
/// The effective set is `(all applicable OR allow)` minus `exclude`; the
/// selection table decides membership.  Omitted selectors select nothing
/// — a present policy never triggers inference.
fn convert(policy: &crate::repository::RepositoryPolicy) -> Policy {
    let selection = policy.selection();
    Policy::Explicit {
        all: selection.all(),
        include: selection
            .allow()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
        exclude: selection
            .exclude()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
    }
}
