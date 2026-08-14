//! One-repository initial-pass state machine and handoff.
//!
//! The initial pass moves one repository through explicit stages; allowed
//! transitions and failure evidence are explicit.  Changed, unchanged,
//! failed, cancelled, and repair-candidate initial results are distinct;
//! no result is final before repair folding (only cancelled is terminal).

#![allow(dead_code)]

use std::{error::Error, fmt};

#[cfg(test)]
mod initial_pass_tests;

/// The explicit initial-pass stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialStage {
    PlanFrozen,
    Acquired,
    Synchronized,
    Failed,
    Cancelled,
}

/// The distinct initial results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialResult {
    Changed,
    Unchanged,
    Failed { reason: String },
    Cancelled,
    RepairCandidate { reason: String },
}

impl InitialResult {
    /// Whether the result is terminal before repair folding.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// The immutable one-repository pass state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialState {
    repository: String,
    plan_identity: String,
    stage: InitialStage,
    result: Option<InitialResult>,
}

/// Transition failures with typed evidence.
#[derive(Debug)]
pub enum TransitionError {
    Invalid {
        from: InitialStage,
        to: InitialStage,
        reason: String,
    },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { from, to, reason } => write!(
                formatter,
                "invalid initial-pass transition {from:?} -> {to:?}: {reason}"
            ),
        }
    }
}
impl Error for TransitionError {}

/// The handoff record for the next stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Handoff {
    pub repository: String,
    pub plan_identity: String,
    pub stage: InitialStage,
}

mod transition;

impl InitialState {
    /// Start the pass with the plan frozen.
    pub fn start(repository: impl Into<String>, plan_identity: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            plan_identity: plan_identity.into(),
            stage: InitialStage::PlanFrozen,
            result: None,
        }
    }

    pub fn stage(&self) -> InitialStage {
        self.stage
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn plan_identity(&self) -> &str {
        &self.plan_identity
    }

    pub fn result(&self) -> &Option<InitialResult> {
        &self.result
    }

    /// Advance with the declared result; the stage follows the result.
    pub fn advance(&self, result: InitialResult) -> Result<Self, TransitionError> {
        let to = match (&self.stage, &result) {
            (InitialStage::PlanFrozen, InitialResult::Unchanged) => InitialStage::Acquired,
            (InitialStage::PlanFrozen, InitialResult::Cancelled) => InitialStage::Cancelled,
            (InitialStage::PlanFrozen, InitialResult::Failed { .. }) => InitialStage::Failed,
            (InitialStage::Acquired, InitialResult::Changed) => InitialStage::Synchronized,
            (InitialStage::Acquired, InitialResult::Unchanged) => InitialStage::Synchronized,
            (InitialStage::Acquired, InitialResult::RepairCandidate { .. }) => {
                InitialStage::Synchronized
            }
            (InitialStage::Acquired, InitialResult::Cancelled) => InitialStage::Cancelled,
            (InitialStage::Acquired, InitialResult::Failed { .. }) => InitialStage::Failed,
            (from, _) => {
                return Err(TransitionError::Invalid {
                    from: *from,
                    to: InitialStage::Failed,
                    reason: "the declared result is not allowed from this stage".to_owned(),
                });
            }
        };
        Ok(Self {
            repository: self.repository.clone(),
            plan_identity: self.plan_identity.clone(),
            stage: to,
            result: Some(result),
        })
    }

    /// The handoff record.
    pub fn handoff(&self) -> Handoff {
        Handoff {
            repository: self.repository.clone(),
            plan_identity: self.plan_identity.clone(),
            stage: self.stage,
        }
    }
}
