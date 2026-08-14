//! Fold the initial and repair histories into one terminal repository
//! outcome.
//!
//! After the initial pass and the bounded repair pass every repository
//! has two histories; the terminal outcome is a deterministic fold:
//! a successful repair upgrades a failure to success, a failed repair
//! keeps the failure with both reasons, an untouched repository keeps
//! its initial outcome, and success and cancellation are terminal.

#![allow(dead_code)]

#[cfg(test)]
mod repair_fold_tests;

use crate::lifecycle::run_summary::RepoOutcome;

/// The repair history of one repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepairHistory {
    /// The repair delivered its scoped commit.
    Succeeded { oid: String },
    /// The repair attempt failed; the typed reason is kept.
    Failed { reason: String },
}

/// Fold the initial outcome and the repair history into ONE terminal
/// repository outcome.  Pure: no I/O, no state.
pub fn fold_into_terminal_outcome(
    initial: &RepoOutcome,
    repair: Option<&RepairHistory>,
) -> RepoOutcome {
    match (initial, repair) {
        (RepoOutcome::Success, _) | (RepoOutcome::Cancelled, _) => initial.clone(),
        (RepoOutcome::Failure { .. }, None) => initial.clone(),
        (RepoOutcome::Failure { reason: _ }, Some(RepairHistory::Succeeded { .. })) => {
            RepoOutcome::Success
        }
        (
            RepoOutcome::Failure { reason },
            Some(RepairHistory::Failed {
                reason: repair_reason,
            }),
        ) => RepoOutcome::Failure {
            reason: format!("{reason}; repair: {repair_reason}"),
        },
    }
}
