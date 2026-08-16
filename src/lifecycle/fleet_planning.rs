//! Per-repository plans with affected naming.
//!
//! Every configured destination gets one immutable plan built from its
//! bound declarations, the source catalog, and its policy.  An
//! unavailable or shadowed source names the affected source and item;
//! a policy failure fails only its destination's plan and peers
//! continue.  Plan order is source precedence then declared order.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_planning_tests;

use crate::configuration::MachineConfiguration;
use crate::lifecycle::fleet_policy::RepositoryPolicyLoad;
use crate::lifecycle::plan_builder::{PlanBuildError, build_repository_plan};
use crate::lifecycle::sync_plan::SyncPlan;
use crate::repository::VerificationCommand;
use crate::source::{ItemDeclaration, SourceCatalog};

/// One destination's plan outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPlan {
    pub repository: String,
    /// The immutable plan, or the typed failure for this destination.
    pub plan: Result<SyncPlan, String>,
    /// The declared verification commands carried from the destination's
    /// policy; empty when the policy is absent, failed, or declares none.
    pub checks: Vec<VerificationCommand>,
}

/// Build every configured destination's plan in declared order.
pub fn build_repository_plans(
    config: &MachineConfiguration,
    catalog: &SourceCatalog,
    declarations: &[(String, Vec<ItemDeclaration>)],
    policies: &[RepositoryPolicyLoad],
) -> Vec<RepositoryPlan> {
    let declared_for = |repository: &str| {
        declarations
            .iter()
            .find(|(name, _)| name == repository)
            .map(|(_, items)| items.as_slice())
            .unwrap_or(&[])
    };
    let policy_for = |repository: &str| {
        policies
            .iter()
            .find(|load| load.repository == repository)
            .map(|load| (&load.policy, &load.failure, load.checks.as_slice()))
    };
    config
        .repositories()
        .iter()
        .map(|destination| {
            let repository = destination.id().as_str().to_owned();
            let checks = policy_for(&repository)
                .map(|(_, _, checks)| checks.to_vec())
                .unwrap_or_default();
            let plan = build_one(
                &repository,
                catalog,
                declared_for(&repository),
                policy_for(&repository),
            );
            RepositoryPlan {
                repository,
                checks,
                plan,
            }
        })
        .collect()
}

fn build_one(
    repository: &str,
    catalog: &SourceCatalog,
    declared: &[ItemDeclaration],
    policy: Option<(
        &Option<crate::lifecycle::plan_selection::Policy>,
        &Option<String>,
        &[VerificationCommand],
    )>,
) -> Result<SyncPlan, String> {
    // A policy load failure fails this destination only.
    if let Some((_, Some(failure), _)) = policy {
        return Err(failure.clone());
    }
    let plan_policy = policy
        .and_then(|(policy, _, _)| policy.as_ref())
        .unwrap_or(&crate::lifecycle::plan_selection::Policy::Absent);
    build_repository_plan(repository, catalog, declared, plan_policy)
        .map_err(|error: PlanBuildError| error.to_string())
}
