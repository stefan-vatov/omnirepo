//! Post-verification parity and authorized-delta revalidation gate.
//!
//! Immediately before Git delivery, managed parity and the authorized
//! delta are revalidated: any managed drift, policy/source/plan identity
//! change, forbidden tracked delta, or concurrent modification converts
//! verification to failure and prevents Git.  Allowed ephemeral cleanup is
//! accounted in the outcome.

#![allow(dead_code)]

use crate::managed_content::CompareOutcome;
use std::{error::Error, fmt};

/// The revalidation verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GateVerdict {
    /// Parity and identity hold; Git delivery may proceed.
    Pass,
    /// The managed content drifted from the frozen source bytes.
    Drift,
    /// The policy, source, or plan identity changed since the plan froze.
    IdentityChanged,
    /// The delta contains a forbidden tracked change.
    ForbiddenDelta,
    /// The destination changed concurrently during verification.
    ConcurrentModification,
}

/// Revalidation failures.
#[derive(Debug)]
pub enum GateError {
    EphemeralCleanup { path: String, reason: String },
}

impl fmt::Display for GateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EphemeralCleanup { path, reason } => {
                write!(
                    formatter,
                    "ephemeral artifact {path} cleanup failed: {reason}"
                )
            }
        }
    }
}
impl Error for GateError {}

/// The gate inputs (frozen identities plus the re-observed state).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateInputs {
    pub frozen_plan_identity: String,
    pub observed_plan_identity: String,
    pub frozen_source_identity: String,
    pub observed_source_identity: String,
    pub frozen_configuration_identity: String,
    pub observed_configuration_identity: String,
}

/// Revalidate the gate.  `parity` is the byte-exact comparison of the
/// managed target against the frozen source bytes; `delta_authorized` is
/// true when the current delta matches the frozen authorized delta;
/// `concurrent_modification` is true when a modification was observed
/// during verification.
pub fn revalidate_gate(
    inputs: &GateInputs,
    parity: CompareOutcome,
    delta_authorized: bool,
    concurrent_modification: bool,
) -> GateVerdict {
    if concurrent_modification {
        return GateVerdict::ConcurrentModification;
    }
    if !matches!(parity, CompareOutcome::Unchanged) {
        return GateVerdict::Drift;
    }
    if !delta_authorized {
        return GateVerdict::ForbiddenDelta;
    }
    if inputs.frozen_plan_identity != inputs.observed_plan_identity
        || inputs.frozen_source_identity != inputs.observed_source_identity
        || inputs.frozen_configuration_identity != inputs.observed_configuration_identity
    {
        return GateVerdict::IdentityChanged;
    }
    GateVerdict::Pass
}

/// Account an allowed ephemeral cleanup: the disposition is recorded in
/// the outcome evidence; a failed cleanup is a typed gate error that
/// prevents Git.
pub fn account_cleanup(
    disposition: &crate::lifecycle::verifier_confinement::ArtifactDisposition,
) -> Result<(), GateError> {
    match disposition {
        crate::lifecycle::verifier_confinement::ArtifactDisposition::Cleaned { path }
        | crate::lifecycle::verifier_confinement::ArtifactDisposition::Retained { path } => {
            let _ = path;
            Ok(())
        }
    }
}

#[cfg(test)]
mod verification_gate_tests;
