//! Frozen verification execution followed by the authorized-delta gate.
//!
//! The frozen verification commands run in declared order with bounded
//! results; any failed check prevents Git delivery.  When all checks pass,
//! the authorized-delta gate is revalidated (parity, delta authorization,
//! identities, concurrent modification); a rejection prevents Git.

#![allow(dead_code)]

use crate::lifecycle::check_runner::{CheckOutcome, CheckResult, run_check};

#[cfg(test)]
mod verify_and_gate_tests;
use crate::lifecycle::command_spec::CommandSpec;
use crate::lifecycle::verification_gate::{GateInputs, GateVerdict, revalidate_gate};
use crate::managed_content::CompareOutcome;
use std::{error::Error, fmt, path::Path, time::Duration};

/// The verification verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationVerdict {
    /// All checks passed and the gate passed: Git delivery may proceed.
    Ready,
    /// At least one frozen check failed: no Git delivery.
    FailedCheck,
    /// Checks passed but the gate rejected the delta: no Git delivery.
    GateRejected,
}

/// The composed run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRun {
    pub checks: Vec<CheckResult>,
    pub verdict: VerificationVerdict,
    pub gate: GateVerdict,
}

/// Run failures.
#[derive(Debug)]
pub enum VerifyError {
    Check { position: usize, reason: String },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Check { position, reason } => {
                write!(
                    formatter,
                    "check at position {position} failed to run: {reason}"
                )
            }
        }
    }
}
impl Error for VerifyError {}

/// Run the frozen verification and revalidate the gate.
pub fn verify_and_gate(
    repository_root: &Path,
    commands: &[CommandSpec],
    gate_inputs: &GateInputs,
    parity: CompareOutcome,
    delta_authorized: bool,
    concurrent_modification: bool,
    budget: Duration,
) -> Result<VerificationRun, VerifyError> {
    let mut checks = Vec::with_capacity(commands.len());
    for command in commands {
        let result =
            run_check(repository_root, command, budget).map_err(|error| VerifyError::Check {
                position: command.position,
                reason: error.to_string(),
            })?;
        checks.push(result);
    }
    let failed = checks
        .iter()
        .any(|check| matches!(check.outcome, CheckOutcome::Failed { .. }));
    if failed {
        return Ok(VerificationRun {
            checks,
            verdict: VerificationVerdict::FailedCheck,
            gate: GateVerdict::Pass,
        });
    }
    let gate = revalidate_gate(
        gate_inputs,
        parity,
        delta_authorized,
        concurrent_modification,
    );
    let verdict = if gate == GateVerdict::Pass {
        VerificationVerdict::Ready
    } else {
        VerificationVerdict::GateRejected
    };
    Ok(VerificationRun {
        checks,
        verdict,
        gate,
    })
}
