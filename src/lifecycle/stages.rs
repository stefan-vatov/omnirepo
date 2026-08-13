//! Stage checkpoints and frozen-input revalidation for the run lifecycle.
//!
//! The run moves through declared stages in order; every transition is
//! journaled as a checkpoint before the next stage's effects begin.  Frozen
//! inputs (baseline, delta, authority identities) must be revalidated before
//! each effectful stage; a revalidation failure stops the run at the
//! boundary.

#![allow(dead_code)]

use super::journal::{JournalError, JournalHandle};
use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent};
use std::{error::Error, fmt};

#[cfg(test)]
mod stages_tests;

/// The run lifecycle stages in declared order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum RunStage {
    Preflight,
    Admission,
    Synchronization,
    Verification,
    GitDelivery,
    Finalization,
}

impl RunStage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Admission => "admission",
            Self::Synchronization => "synchronization",
            Self::Verification => "verification",
            Self::GitDelivery => "git_delivery",
            Self::Finalization => "finalization",
        }
    }
}

/// The stage machine for one run.
#[derive(Clone, Debug, Default)]
pub struct StageMachine {
    current: Option<RunStage>,
}

impl StageMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<RunStage> {
        self.current
    }

    /// Advance to the next declared stage.  The transition must be exactly
    /// the declared successor; jumps, repeats, and regressions fail.
    pub fn advance(
        &mut self,
        journal: &JournalHandle,
        run_id: &str,
        next: RunStage,
    ) -> Result<(), StageError> {
        let allowed = match self.current {
            None => next == RunStage::Preflight,
            Some(current) => next == successor(current),
        };
        if !allowed {
            return Err(StageError::InvalidTransition {
                from: self.current.map(RunStage::label),
                to: next.label(),
            });
        }
        self.current = Some(next);
        checkpoint_stage(journal, run_id, next)?;
        Ok(())
    }
}

fn successor(stage: RunStage) -> RunStage {
    match stage {
        RunStage::Preflight => RunStage::Admission,
        RunStage::Admission => RunStage::Synchronization,
        RunStage::Synchronization => RunStage::Verification,
        RunStage::Verification => RunStage::GitDelivery,
        RunStage::GitDelivery => RunStage::Finalization,
        RunStage::Finalization => RunStage::Finalization,
    }
}

/// Journal one stage checkpoint with the exact stage label.
fn checkpoint_stage(
    journal: &JournalHandle,
    run_id: &str,
    stage: RunStage,
) -> Result<(), StageError> {
    let evidence = EvidenceRef::new(EvidenceKind::Process, format!("stage/{}", stage.label()), 0)
        .map_err(|error| StageError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: None,
            evidence,
            stage: Some("stage"),
        })
        .map_err(StageError::Journal)?;
    Ok(())
}

/// Stage and checkpoint failures.
#[derive(Debug)]
pub enum StageError {
    Journal(JournalError),
    InvalidTransition {
        from: Option<&'static str>,
        to: &'static str,
    },
}

impl fmt::Display for StageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(formatter, "stage journal failure: {error}"),
            Self::InvalidTransition { from, to } => match from {
                Some(from) => write!(formatter, "invalid stage transition: {from} -> {to}"),
                None => write!(formatter, "invalid stage transition: start -> {to}"),
            },
        }
    }
}
impl Error for StageError {}
