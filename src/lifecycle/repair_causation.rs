//! Current-run causation proof from baseline and frozen lineage.
//!
//! A failure is causally linked to the current run only when the observed
//! baseline identity matches the frozen lineage AND the sync/verification
//! effect was durably recorded in this run.  Anything else — a lineage
//! mismatch, a missing effect, or an empty baseline — is not proven and
//! never proceeds to repair.

#![allow(dead_code)]

use std::{error::Error, fmt};

#[cfg(test)]
mod repair_causation_tests;

/// The causation verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CausationVerdict {
    /// The failure is causally linked to the current run.
    Proven,
    /// Causation is not proven; the typed reason explains why.
    NotProven { reason: String },
}

/// Causation proof failures.
#[derive(Debug)]
pub enum CausationError {
    EmptyBaseline,
}

impl fmt::Display for CausationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBaseline => write!(formatter, "the baseline identity is empty"),
        }
    }
}
impl Error for CausationError {}

/// Prove current-run causation from the observed baseline identity, the
/// frozen lineage identity, and the durable effect record.  Pure: no I/O,
/// no state.
pub fn prove_current_run_causation(
    baseline_identity: &str,
    frozen_lineage_identity: &str,
    effect_recorded: bool,
) -> CausationVerdict {
    if baseline_identity.is_empty() {
        return CausationVerdict::NotProven {
            reason: "the baseline identity is empty".to_owned(),
        };
    }
    if baseline_identity != frozen_lineage_identity {
        return CausationVerdict::NotProven {
            reason: format!(
                "baseline {baseline_identity:?} does not match the frozen lineage {frozen_lineage_identity:?}"
            ),
        };
    }
    if !effect_recorded {
        return CausationVerdict::NotProven {
            reason: "no sync or verification effect was durably recorded in this run".to_owned(),
        };
    }
    CausationVerdict::Proven
}
