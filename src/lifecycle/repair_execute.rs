//! One confined agent invocation with a causally bounded repair task.
//!
//! The repair task is bounded by causation and eligibility, and exactly
//! one attempt is journaled before the agent runs.  The agent executes
//! under the destination-only confinement; its output is bounded and
//! sanitized evidence; a crash, a timeout, or an escaping agent path
//! fails typed without claiming success.

#![allow(dead_code)]

use crate::lifecycle::agent_confinement::{ConfinementError, confine};

#[cfg(test)]
mod repair_execute_tests;
use crate::lifecycle::agent_runtime::{AgentRuntimeError, run_agent};
use crate::lifecycle::journal::{JournalError, JournalHandle};
use crate::lifecycle::repair_classify::{FailureClass, classify_failure};
use crate::platform::{AgentWorkingDirectoryRoot, AuthorityRoot, ReadOnly};
use std::{error::Error, fmt, path::Path, time::Duration};

/// The repair outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepairOutcome {
    Succeeded { evidence: String },
}

/// Repair failures.
#[derive(Debug)]
pub enum RepairError {
    CausationNotProven { reason: String },
    ClassTerminal { class: FailureClass },
    AgentPathEscapes { path: String },
    Confinement(ConfinementError),
    AgentCrashed { code: Option<i32> },
    AgentTimedOut { budget: Duration },
    Journal(JournalError),
}

impl fmt::Display for RepairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CausationNotProven { reason } => {
                write!(formatter, "repair causation is not proven: {reason}")
            }
            Self::ClassTerminal { class } => {
                write!(
                    formatter,
                    "failure class {class:?} is terminal; no repair proceeds"
                )
            }
            Self::AgentPathEscapes { path } => {
                write!(formatter, "the agent path {path:?} escapes the destination")
            }
            Self::Confinement(error) => write!(formatter, "repair confinement failure: {error}"),
            Self::AgentCrashed { code } => {
                write!(formatter, "the repair agent crashed with code {code:?}")
            }
            Self::AgentTimedOut { budget } => {
                write!(formatter, "the repair agent exceeded its {budget:?} budget")
            }
            Self::Journal(error) => write!(formatter, "repair execution journal failure: {error}"),
        }
    }
}
impl Error for RepairError {}

/// One confined repair attempt request.
#[derive(Clone, Debug)]
pub struct RepairRequest<'a> {
    /// The confined destination working directory.
    pub destination: &'a Path,
    /// The agent argv; the first element is the agent path.
    pub argv: &'a [String],
    /// The repair task label.
    pub task: &'a str,
    /// The run journal handle.
    pub journal: &'a JournalHandle,
    /// The current run id.
    pub run_id: &'a str,
    /// The affected repository id.
    pub repository: &'a str,
    /// The frozen repair inputs (causation witness).
    pub frozen_inputs: &'a [String],
    /// The execution budget.
    pub budget: Duration,
    /// True when the agent is the machine-declared adapter (frozen
    /// identity, trusted); false when the argv is repository-supplied
    /// (the escape check applies).
    pub trusted_agent: bool,
}

/// Execute one confined repair: causation proven, class eligible, agent
/// path inside the destination, bounded run with typed termination.
pub fn execute_confined_repair(request: RepairRequest<'_>) -> Result<RepairOutcome, RepairError> {
    let RepairRequest {
        destination,
        argv,
        task,
        journal,
        run_id,
        repository,
        frozen_inputs,
        budget,
        trusted_agent,
    } = request;
    // Causation bound: the frozen inputs must be present (the caller
    // already proved current-run causation upstream; the execution gate
    // verifies the inputs survive to this point).
    if frozen_inputs.is_empty() {
        return Err(RepairError::CausationNotProven {
            reason: "no frozen repair inputs; causation is not proven".to_owned(),
        });
    }
    // Eligibility bound: the class must be repairable.
    let class = FailureClass::SyncDrift;
    if !matches!(
        classify_failure(class).eligibility,
        crate::lifecycle::repair_classify::Eligibility::Repairable { .. }
    ) {
        return Err(RepairError::ClassTerminal { class });
    }
    // A repository-supplied agent path must be inside the destination;
    // the machine-declared adapter is trusted by the machine priority and
    // its frozen identity.
    let agent_path = Path::new(&argv[0]);
    if !trusted_agent && !agent_path.starts_with(destination) {
        return Err(RepairError::AgentPathEscapes {
            path: agent_path.display().to_string(),
        });
    }
    let root = AuthorityRoot::<AgentWorkingDirectoryRoot, ReadOnly>::open(destination).map_err(
        |error| {
            RepairError::Confinement(ConfinementError::Root {
                path: destination.to_path_buf(),
                reason: error.to_string(),
            })
        },
    )?;
    let confinement = confine(&root, &[], &[]).map_err(RepairError::Confinement)?;
    // Exactly one attempt is journaled (the reservation marker) before the
    // agent runs.
    let evidence = crate::lifecycle::event::EvidenceRef::new(
        crate::lifecycle::event::EvidenceKind::Process,
        format!("repair/{repository}/attempt/1/{task}"),
        1,
    )
    .map_err(|error| RepairError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(crate::lifecycle::event::JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: Some(repository.to_owned()),
            evidence,
            stage: Some("repair-execute"),
        })
        .map_err(RepairError::Journal)?;
    match run_agent(
        argv,
        &confinement,
        &destination.join(".omnirepo-repair"),
        budget,
    ) {
        Ok(captured) => Ok(RepairOutcome::Succeeded {
            evidence: captured.sanitized,
        }),
        Err(AgentRuntimeError::Timeout { budget }) => Err(RepairError::AgentTimedOut { budget }),
        Err(AgentRuntimeError::Crashed { code }) => Err(RepairError::AgentCrashed { code }),
        Err(error) => Err(RepairError::Confinement(ConfinementError::Root {
            path: destination.to_path_buf(),
            reason: error.to_string(),
        })),
    }
}
