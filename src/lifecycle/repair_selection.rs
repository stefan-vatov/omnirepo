//! Select eligible failed repositories after the complete initial pass.
//!
//! A failed repository is eligible for bounded repair only when BOTH
//! conditions hold: its failure class is repairable (never a terminal
//! class) AND current-run causation is proven.  The selection is pure,
//! deterministic, and keeps the input order; only eligible repositories
//! are returned.

#![allow(dead_code)]

#[cfg(test)]
mod repair_selection_tests;

use crate::lifecycle::repair_causation::CausationVerdict;
use crate::lifecycle::repair_classify::{Eligibility, FailureClass, classify_failure};

/// One failed repository from the completed initial pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedRepository {
    pub repository: String,
    pub class: FailureClass,
}

/// One selected repair target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EligibleRepair {
    pub repository: String,
    pub class: FailureClass,
    pub attempts: u8,
    pub reason: String,
}

/// Select the eligible failed repositories.
///
/// Repairable class AND proven causation, in input order.  Terminal
/// classes, unproven causation, and missing causation evidence are
/// excluded.
pub fn select_eligible_failed(
    failed: &[FailedRepository],
    causation: &[(String, CausationVerdict)],
) -> Vec<EligibleRepair> {
    let mut selected = Vec::new();
    for failed in failed {
        let classification = classify_failure(failed.class);
        if !matches!(classification.eligibility, Eligibility::Repairable { .. }) {
            continue;
        }
        let Eligibility::Repairable { attempts } = classification.eligibility else {
            continue;
        };
        let proven = matches!(
            causation
                .iter()
                .find(|(id, _)| id == &failed.repository)
                .map(|(_, verdict)| verdict),
            Some(CausationVerdict::Proven)
        );
        if !proven {
            continue;
        }
        selected.push(EligibleRepair {
            repository: failed.repository.clone(),
            class: failed.class,
            attempts,
            reason: classification.explanation,
        });
    }
    selected
}
