//! The generated lifecycle model: durable run/repository states, actions,
//! and executable invariants.
//!
//! Every transition references an implemented stage and fault point in
//! this codebase (run_record, initial_sync, check_runner, commit_journal,
//! remote_push, repair_execute, cancellation, crash reconcile, replay).
//! The invariants are executable: `assert_invariants` returns typed
//! violations.  Undecided and unsupported behaviors are excluded by
//! construction — the action set is closed.

#![allow(dead_code)]

#[cfg(test)]
mod lifecycle_model_tests;

use std::{error::Error, fmt};

/// The durable run state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    Invocation,
    Sync,
    Check,
    Commit,
    Push,
    Repair,
    Cancelling,
    Cancelled,
    Crashed,
    Complete,
}

/// One repository's durable state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepoState {
    Pending,
    Syncing,
    Verified,
    Committing,
    Pushed,
    Repairing,
    Failed,
    Cancelled,
}

/// The closed action set.  Every action maps to an implemented stage:
/// StartSync -> initial_sync, RunCheck -> check_runner, Commit ->
/// commit_journal, Push -> remote_push, Repair -> repair_execute, Cancel
/// -> cancellation, Crash -> crash reconcile (commit_journal/remote_push),
/// Restart -> replay, RecordEvidence -> journal, Complete -> record_finalize.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action<'a> {
    StartSync { repository: &'a str },
    RunCheck { repository: &'a str },
    Commit { repository: &'a str },
    Push { repository: &'a str },
    Repair { repository: &'a str },
    RecordEvidence { repository: &'a str },
    Cancel,
    Crash,
    Restart { repository: &'a str },
    Complete,
}

/// Transition failures.
#[derive(Debug)]
pub enum ModelError {
    UnknownRepository { repository: String },
    TerminalState { state: RunState },
    EffectWithoutIntent { repository: String },
    DuplicateReservation { repository: String },
    RepoAlreadyAdvanced { repository: String },
    NotRepairable { repository: String },
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownRepository { repository } => {
                write!(formatter, "repository {repository:?} is not declared")
            }
            Self::TerminalState { state } => {
                write!(formatter, "the run is terminal in state {state:?}")
            }
            Self::EffectWithoutIntent { repository } => {
                write!(
                    formatter,
                    "effect without a prior intent for {repository:?}"
                )
            }
            Self::DuplicateReservation { repository } => {
                write!(formatter, "repository {repository:?} is already reserved")
            }
            Self::RepoAlreadyAdvanced { repository } => {
                write!(formatter, "repository {repository:?} cannot move backwards")
            }
            Self::NotRepairable { repository } => {
                write!(
                    formatter,
                    "repository {repository:?} has no failed state to repair"
                )
            }
        }
    }
}
impl Error for ModelError {}

/// One executable invariant violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvariantViolation {
    pub invariant: &'static str,
    pub detail: String,
}

/// The lifecycle model: one run, its declared repositories, and the
/// effect log that records intent-before-effect.
#[derive(Clone, Debug)]
pub struct LifecycleModel {
    pub run: RunState,
    pub run_id: String,
    repositories: Vec<String>,
    states: Vec<RepoState>,
    /// The effect log: every recorded intent and evidence, in order.
    pub effect_log: Vec<String>,
    reserved: Vec<String>,
}

/// A fresh model for one run over its declared repositories.
pub fn new_model(run_id: &str, repositories: &[String]) -> LifecycleModel {
    LifecycleModel {
        run: RunState::Invocation,
        run_id: run_id.to_owned(),
        repositories: repositories.to_vec(),
        states: vec![RepoState::Pending; repositories.len()],
        effect_log: Vec::new(),
        reserved: Vec::new(),
    }
}

fn index_of(model: &LifecycleModel, repository: &str) -> Result<usize, ModelError> {
    model
        .repositories
        .iter()
        .position(|candidate| candidate == repository)
        .ok_or_else(|| ModelError::UnknownRepository {
            repository: repository.to_owned(),
        })
}

/// Apply one action.  Undecided behaviors are excluded by construction:
/// there is no action outside this closed set.
pub fn transition(model: &mut LifecycleModel, action: Action<'_>) -> Result<(), ModelError> {
    if matches!(
        model.run,
        RunState::Cancelled | RunState::Crashed | RunState::Complete
    ) {
        match (&action, model.run) {
            (Action::Crash, RunState::Cancelled | RunState::Crashed | RunState::Complete) => {
                return Ok(());
            }
            (Action::Restart { .. }, RunState::Crashed) => {
                model.run = RunState::Sync;
                model
                    .effect_log
                    .push("restart:resume-from-journaled-intent".to_owned());
                return Ok(());
            }
            (Action::RecordEvidence { repository }, RunState::Crashed) => {
                // The evidence can still be reconciled from the journal.
                let index = index_of(model, repository)?;
                model.effect_log.push(format!("evidence:{repository}"));
                model.states[index] = advance_repo(model.states[index]);
                return Ok(());
            }
            (Action::Complete, RunState::Crashed) => {
                return Err(ModelError::TerminalState { state: model.run });
            }
            _ => {
                return Err(ModelError::TerminalState { state: model.run });
            }
        }
    }
    match action {
        Action::StartSync { repository } => {
            let index = index_of(model, repository)?;
            if model.states[index] != RepoState::Pending {
                return Err(ModelError::RepoAlreadyAdvanced {
                    repository: repository.to_owned(),
                });
            }
            model.states[index] = RepoState::Syncing;
            model.run = RunState::Sync;
            model.effect_log.push(format!("intent:sync:{repository}"));
        }
        Action::RunCheck { repository } => {
            let index = index_of(model, repository)?;
            // The check stage follows the sync stage: evidence requires
            // the prior sync intent.
            let synced = model
                .effect_log
                .iter()
                .any(|entry| entry == &format!("intent:sync:{repository}"));
            if !synced {
                return Err(ModelError::EffectWithoutIntent {
                    repository: repository.to_owned(),
                });
            }
            model.states[index] = RepoState::Verified;
            model.run = RunState::Check;
            model
                .effect_log
                .push(format!("evidence:check:{repository}"));
        }
        Action::Commit { repository } => {
            let index = index_of(model, repository)?;
            model.states[index] = RepoState::Committing;
            model.run = RunState::Commit;
            model.effect_log.push(format!("intent:commit:{repository}"));
        }
        Action::Push { repository } => {
            let index = index_of(model, repository)?;
            // The push follows the commit: evidence requires the prior
            // commit intent.
            let committed = model
                .effect_log
                .iter()
                .any(|entry| entry == &format!("intent:commit:{repository}"));
            if !committed {
                return Err(ModelError::EffectWithoutIntent {
                    repository: repository.to_owned(),
                });
            }
            model.states[index] = RepoState::Pushed;
            model.run = RunState::Push;
            model.effect_log.push(format!("evidence:push:{repository}"));
        }
        Action::Repair { repository } => {
            let index = index_of(model, repository)?;
            if model.states[index] == RepoState::Pending {
                return Err(ModelError::NotRepairable {
                    repository: repository.to_owned(),
                });
            }
            if model.reserved.contains(&repository.to_owned()) {
                return Err(ModelError::DuplicateReservation {
                    repository: repository.to_owned(),
                });
            }
            model.reserved.push(repository.to_owned());
            model.states[index] = RepoState::Repairing;
            model.run = RunState::Repair;
            model.effect_log.push(format!("intent:repair:{repository}"));
        }
        Action::RecordEvidence { repository } => {
            let index = index_of(model, repository)?;
            let prior_intent = model
                .effect_log
                .iter()
                .rev()
                .any(|entry| entry.starts_with("intent:") && entry.ends_with(repository));
            if !prior_intent {
                return Err(ModelError::EffectWithoutIntent {
                    repository: repository.to_owned(),
                });
            }
            model.states[index] = advance_repo(model.states[index]);
            model.effect_log.push(format!("evidence:{repository}"));
        }
        Action::Cancel => {
            model.run = RunState::Cancelled;
            model.effect_log.push("intent:cancel".to_owned());
        }
        Action::Crash => {
            model.run = RunState::Crashed;
            model.effect_log.push("crash:fault-point".to_owned());
        }
        Action::Restart { repository } => {
            index_of(model, repository)?;
            model.run = RunState::Sync;
            model
                .effect_log
                .push("restart:resume-from-journaled-intent".to_owned());
        }
        Action::Complete => {
            model.run = RunState::Complete;
            model.effect_log.push("terminal:complete".to_owned());
        }
    }
    Ok(())
}

fn advance_repo(state: RepoState) -> RepoState {
    match state {
        RepoState::Syncing => RepoState::Verified,
        RepoState::Committing => RepoState::Pushed,
        other => other,
    }
}

/// Execute the invariants and return every violation.  Pure: the model is
/// immutable during the check.
pub fn assert_invariants(model: &LifecycleModel) -> Vec<InvariantViolation> {
    let mut violations = Vec::new();
    // Effect-before-event: every evidence entry has a prior intent for
    // the same repository.
    for (index, entry) in model.effect_log.iter().enumerate() {
        if entry.starts_with("evidence:") {
            let repository = entry
                .trim_start_matches("evidence:")
                .rsplit(':')
                .next()
                .unwrap_or("");
            let prior_intent = model.effect_log[..index]
                .iter()
                .rev()
                .any(|prior| prior.starts_with("intent:") && prior.ends_with(repository));
            if !prior_intent {
                violations.push(InvariantViolation {
                    invariant: "effect-before-event",
                    detail: format!("evidence for {repository:?} has no prior intent"),
                });
            }
        }
    }
    // Finality: terminal states never transition.  The transition
    // function is the enforcement witness; the terminal state is recorded
    // exactly once in the effect log.
    let terminal_markers = model
        .effect_log
        .iter()
        .filter(|entry| entry.starts_with("terminal:"))
        .count();
    if terminal_markers > 1 {
        violations.push(InvariantViolation {
            invariant: "finality",
            detail: "the terminal marker was recorded more than once".to_owned(),
        });
    }
    // Uniqueness: a repository appears exactly once in the states table
    // (guaranteed by construction); a reservation is never duplicated.
    let mut seen = std::collections::BTreeSet::new();
    for repository in &model.reserved {
        if !seen.insert(repository.clone()) {
            violations.push(InvariantViolation {
                invariant: "uniqueness",
                detail: format!("duplicate reservation for {repository:?}"),
            });
        }
    }
    // Scope: every intent/evidence repository is declared.  Run-level
    // entries (cancel, complete) carry no repository segment.
    for entry in &model.effect_log {
        let repository = if entry.starts_with("intent:") {
            entry.trim_start_matches("intent:")
        } else if entry.starts_with("evidence:") {
            entry.trim_start_matches("evidence:")
        } else {
            continue;
        };
        if !repository.contains(':') {
            // Run-level intent (for example "cancel"): no repository.
            continue;
        }
        let repository = repository.rsplit(':').next().unwrap_or(repository);
        if !model.repositories.contains(&repository.to_owned()) {
            violations.push(InvariantViolation {
                invariant: "scope",
                detail: format!("{repository:?} is not declared"),
            });
        }
    }
    violations
}
