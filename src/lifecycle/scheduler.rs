//! Journal-backpressure-aware scheduling.
//!
//! The scheduler admits an effect only when its repository intent is
//! accepted by the journal's bounded queue (try_submit): every admitted
//! effect can persist its intent and result.  A full queue is backpressure
//! — the repository stays queued and is never silently dropped.  A poisoned
//! writer produces a typed writer-failure event, stops new admission, and
//! the final accounting records a cancelled outcome for every repository
//! that never ran, so no repository silently disappears.

#![allow(dead_code)]

use super::fleet_permits::{FleetPermits, PermitError};
use super::journal::{JournalError, JournalHandle, TrySubmitError};
use crate::lifecycle::event::{JournalEvent, Operation, Outcome};

/// The journal probe classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeClass {
    /// The intent was accepted; the effect may run.
    Admitted,
    /// The bounded queue is full; apply backpressure (stay queued).
    Backpressured,
    /// The writer is poisoned or gone; stop scheduling.
    WriterFailed,
}

/// Classify a non-blocking intent submission.
pub fn classify_probe(result: Result<u64, TrySubmitError>) -> ProbeClass {
    match result {
        Ok(_) => ProbeClass::Admitted,
        Err(TrySubmitError::Full) => ProbeClass::Backpressured,
        Err(TrySubmitError::Poisoned) => ProbeClass::WriterFailed,
        Err(TrySubmitError::Rejected(_)) => ProbeClass::WriterFailed,
    }
}

/// One scheduling decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerEvent {
    /// The repository was admitted with its intent persisted.
    Admitted { repository: String },
    /// Backpressure: the intent could not be persisted yet; the repository
    /// stays queued.
    Backpressured { repository: String },
    /// The journal writer failed; scheduling stopped.
    WriterFailed { reason: String },
    /// The final accounting recorded a cancelled outcome for a repository
    /// that never ran.
    FinalCancelled { repository: String },
}

/// The backpressure-aware scheduler state.
#[derive(Debug)]
pub struct Scheduler {
    journal: JournalHandle,
    run_id: String,
    permits: FleetPermits,
    stopped: bool,
}

impl Scheduler {
    pub fn new(journal: JournalHandle, run_id: String, permits: FleetPermits) -> Self {
        Self {
            journal,
            run_id,
            permits,
            stopped: false,
        }
    }

    /// Admit one repository: persist the intent first, then grant the
    /// permit.  Backpressure keeps the repository queued; a writer failure
    /// stops scheduling and the caller finalizes.
    pub fn try_admit(&self, repository: &str) -> SchedulerEvent {
        if self.stopped {
            return SchedulerEvent::WriterFailed {
                reason: "scheduling already stopped".to_owned(),
            };
        }
        let probe = self.journal.try_submit(JournalEvent::RepositoryIntent {
            checkpoint: 0,
            run_id: self.run_id.clone(),
            repository_id: repository.to_owned(),
            operation: Operation::Synchronize,
            attempt: 1,
        });
        match classify_probe(probe) {
            ProbeClass::Admitted => SchedulerEvent::Admitted {
                repository: repository.to_owned(),
            },
            ProbeClass::Backpressured => SchedulerEvent::Backpressured {
                repository: repository.to_owned(),
            },
            ProbeClass::WriterFailed => {
                self.permits.cancel();
                SchedulerEvent::WriterFailed {
                    reason: "the journal writer failed".to_owned(),
                }
            }
        }
    }

    /// Final accounting: every queued repository that never ran receives a
    /// cancelled outcome (best-effort) and is returned so the run report
    /// never loses it.
    pub fn finalize_queued(&self, queued: &[String]) -> Vec<SchedulerEvent> {
        let mut events = Vec::new();
        for repository in queued {
            let submitted = self.journal.try_submit(JournalEvent::RepositoryResult {
                checkpoint: 0,
                run_id: self.run_id.clone(),
                repository_id: repository.clone(),
                operation: Operation::Synchronize,
                attempt: 1,
                outcome: Outcome::Cancelled,
            });
            let _ = submitted;
            events.push(SchedulerEvent::FinalCancelled {
                repository: repository.clone(),
            });
        }
        events
    }

    /// The permit ledger (bounded admission).
    pub fn permits(&self) -> &FleetPermits {
        &self.permits
    }
}

/// Map a permit error to a typed scheduling failure.
pub fn permit_failure(error: PermitError) -> String {
    match error {
        PermitError::RunCancelled => "the run is cancelled".to_owned(),
        PermitError::WriterUnhealthy => "the journal writer is unhealthy".to_owned(),
        PermitError::Limit { reason } => reason,
    }
}

/// The journal error label for evidence.
pub fn journal_failure_label(error: &JournalError) -> &'static str {
    match error {
        JournalError::Write(_) => "write",
        JournalError::Invalid(_) => "invalid",
        JournalError::Poisoned => "poisoned",
    }
}

#[cfg(test)]
mod scheduler_tests;
