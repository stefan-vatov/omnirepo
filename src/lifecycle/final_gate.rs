//! The complete acceptance gate from the canonical journey matrix.
//!
//! The final gate runs every canonical journey (from the journey
//! matrix) and every normative quality gate over the complete product
//! surface, with no averaging: each journey and each gate must pass,
//! and every failure is reported explicitly.  The verdict is all-pass
//! or a named set of failures.

#![allow(dead_code)]

#[cfg(test)]
mod final_gate_tests;

use crate::lifecycle::acceptance_journeys::{
    JourneyOutcome, canonical_journey_matrix, run_journey,
};
use crate::lifecycle::release_gates::{GateRun, run_normative_gates};
use std::path::Path;

/// One journey result in the final gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JourneyResult {
    pub id: &'static str,
    pub outcome: String,
}

/// The final gate report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalGateReport {
    pub journeys: Vec<JourneyResult>,
    pub gates: Vec<GateRun>,
    pub all_passed: bool,
}

/// Run every canonical journey from the clean environment and the
/// supplied quality gates (the CI invokes the normative cargo gates;
/// the unit tests supply fixture gates — cargo-in-cargo recursion is
/// never exercised by the tests).
pub fn run_final_acceptance_gate(
    clean_home: &Path,
    gates: &[(String, Vec<String>)],
) -> FinalGateReport {
    let matrix = canonical_journey_matrix();
    let journeys = matrix
        .iter()
        .map(|journey| {
            let outcome = match run_journey(journey, clean_home) {
                Ok(report) => match report.outcome {
                    JourneyOutcome::Passed => "passed",
                    JourneyOutcome::Failed { .. } => "failed",
                },
                Err(_) => "failed",
            };
            JourneyResult {
                id: journey.id,
                outcome: outcome.to_owned(),
            }
        })
        .collect::<Vec<_>>();
    let gates = run_quality_gates(gates);
    let all_passed = journeys.iter().all(|journey| journey.outcome == "passed")
        && gates.iter().all(|gate| gate.passed);
    FinalGateReport {
        journeys,
        gates,
        all_passed,
    }
}

/// Run the quality gates without averaging: every failure is reported.
pub fn run_quality_gates(gates: &[(String, Vec<String>)]) -> Vec<GateRun> {
    run_normative_gates(gates)
}

/// The normative gate commands over the repository (explicit argv).
fn normative_gates() -> Vec<(String, Vec<String>)> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let arg = |value: &str| value.to_owned();
    vec![
        (
            "fmt".to_owned(),
            vec![
                cargo.clone(),
                arg("fmt"),
                arg("--all"),
                arg("--"),
                arg("--check"),
            ],
        ),
        (
            "clippy".to_owned(),
            vec![
                cargo.clone(),
                arg("clippy"),
                arg("--workspace"),
                arg("--all-targets"),
                arg("--all-features"),
                arg("--locked"),
                arg("--"),
                arg("-D"),
                arg("warnings"),
            ],
        ),
        (
            "test".to_owned(),
            vec![
                cargo.clone(),
                arg("test"),
                arg("--workspace"),
                arg("--all-targets"),
                arg("--all-features"),
                arg("--locked"),
            ],
        ),
        (
            "doc".to_owned(),
            vec![
                cargo.clone(),
                arg("test"),
                arg("--workspace"),
                arg("--doc"),
                arg("--all-features"),
                arg("--locked"),
            ],
        ),
        (
            "build".to_owned(),
            vec![
                cargo,
                arg("build"),
                arg("--workspace"),
                arg("--all-targets"),
                arg("--all-features"),
                arg("--locked"),
            ],
        ),
    ]
}
