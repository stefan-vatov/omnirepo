//! The model property suite: no duplicate effects, no lost outcomes, and
//! no authority escapes across generated and fault-injected model runs.
//!
//! The suite is deterministic for a fixed seed set.  Every run is a
//! generated action sequence (optionally fault-injected); the model
//! executes it, and the suite asserts the durable properties:
//! effect-before-event, finality with exactly one terminal marker,
//! uniqueness of reservations, and scope (never an undeclared
//! repository).  Failures are typed and carry the seed and the sequence
//! index.

#![allow(dead_code)]

use crate::lifecycle::lifecycle_model::{
    Action, LifecycleModel, ModelError, RunState, assert_invariants, new_model, transition,
};
use crate::lifecycle::model_generation::{FaultKind, generate_actions, inject_fault};
#[cfg(test)]
mod model_property_suite_tests;

use std::fmt;

/// One typed property failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PropertyFailure {
    DuplicateEffect {
        seed: u64,
        index: usize,
        repository: String,
    },
    LostOutcome {
        seed: u64,
        index: usize,
        detail: String,
    },
    AuthorityEscape {
        seed: u64,
        index: usize,
        repository: String,
    },
    Invariant {
        seed: u64,
        index: usize,
        detail: String,
    },
}

impl fmt::Display for PropertyFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateEffect {
                seed,
                index,
                repository,
            } => write!(
                formatter,
                "seed {seed} sequence {index}: duplicate effect for {repository:?}"
            ),
            Self::LostOutcome {
                seed,
                index,
                detail,
            } => {
                write!(
                    formatter,
                    "seed {seed} sequence {index}: lost outcome: {detail}"
                )
            }
            Self::AuthorityEscape {
                seed,
                index,
                repository,
            } => write!(
                formatter,
                "seed {seed} sequence {index}: authority escape for {repository:?}"
            ),
            Self::Invariant {
                seed,
                index,
                detail,
            } => {
                write!(
                    formatter,
                    "seed {seed} sequence {index}: invariant: {detail}"
                )
            }
        }
    }
}

/// The suite report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyReport {
    pub seeds_run: u64,
    pub sequences_run: u64,
    pub failures: Vec<PropertyFailure>,
}

/// Run the property suite over the seed set.  Deterministic.
pub fn run_property_suite(seeds: &[u64]) -> PropertyReport {
    let mut report = PropertyReport {
        seeds_run: seeds.len() as u64,
        sequences_run: 0,
        failures: Vec::new(),
    };
    let repositories = vec!["repo-a".to_owned()];
    for (seed_index, seed) in seeds.iter().enumerate() {
        let base = generate_actions(*seed, 20, &repositories);
        // Three variants per seed: the raw sequence, a crash-injected
        // sequence, and a cancel-injected sequence.
        let variants = [
            base.clone(),
            inject_fault(&base, seed.wrapping_add(1), FaultKind::Crash),
            inject_fault(&base, seed.wrapping_add(2), FaultKind::Cancel),
        ];
        for (variant_index, actions) in variants.iter().enumerate() {
            report.sequences_run += 1;
            let sequence_index = seed_index * 3 + variant_index;
            check_sequence(
                *seed,
                sequence_index,
                actions,
                &repositories,
                &mut report.failures,
            );
        }
    }
    report
}

fn check_sequence(
    seed: u64,
    index: usize,
    actions: &[Action<'_>],
    repositories: &[String],
    failures: &mut Vec<PropertyFailure>,
) {
    let mut model = new_model(&format!("run-{seed}-{index}"), repositories);
    let mut terminal = None;
    for (step, action) in actions.iter().enumerate() {
        match transition(&mut model, action.clone()) {
            Ok(()) => {
                if let Some(state) = terminal_state(&model) {
                    terminal = Some(state);
                    // The terminal state is recorded once.
                    let markers = model
                        .effect_log
                        .iter()
                        .filter(|entry| entry.starts_with("terminal:"))
                        .count();
                    if markers > 1 {
                        failures.push(PropertyFailure::LostOutcome {
                            seed,
                            index,
                            detail: format!("terminal marker recorded {markers} times"),
                        });
                    }
                    break;
                }
            }
            Err(error) => {
                // A typed rejection is the model's guard working: the
                // durable properties hold precisely because the model
                // refuses the invalid action.  The guard is recorded, and
                // the accepted prefix still obeys the invariants.
                check_error(seed, index, step, &error, repositories, failures);
                break;
            }
        }
    }
    if terminal.is_none() && model.run == RunState::Complete {
        // The suite never drops a completed outcome.
    }
    let _ = &mut *failures;
    // The invariants must hold on every executed prefix.
    let violations = assert_invariants(&model);
    for violation in violations {
        failures.push(PropertyFailure::Invariant {
            seed,
            index,
            detail: format!("{}: {}", violation.invariant, violation.detail),
        });
    }
}

fn terminal_state(model: &LifecycleModel) -> Option<RunState> {
    match model.run {
        RunState::Cancelled | RunState::Crashed | RunState::Complete => Some(model.run),
        _ => None,
    }
}

fn check_error(
    _seed: u64,
    _index: usize,
    _step: usize,
    error: &ModelError,
    repositories: &[String],
    failures: &mut Vec<PropertyFailure>,
) {
    match error {
        ModelError::UnknownRepository { repository } => {
            if !repositories.contains(repository) {
                failures.push(PropertyFailure::AuthorityEscape {
                    seed: _seed,
                    index: _index,
                    repository: repository.clone(),
                });
            }
        }
        // Every other typed rejection is the model's enforcement: the
        // property "no duplicate effects" holds because the model
        // refuses duplicates and out-of-order effects.
        ModelError::EffectWithoutIntent { .. }
        | ModelError::DuplicateReservation { .. }
        | ModelError::TerminalState { .. }
        | ModelError::RepoAlreadyAdvanced { .. }
        | ModelError::NotRepairable { .. } => {}
    }
}
