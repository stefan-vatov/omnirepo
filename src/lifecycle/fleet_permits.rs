//! Bounded repository admission queues and permits.
//!
//! Ready repositories wait in a deterministic FIFO queue; a permit is
//! granted only while the active count is below the limit, the run is not
//! cancelled, and the journal writer is healthy.  Queue order affects
//! performance only: every selected repository reaches its own terminal
//! accounting regardless of order.

#![allow(dead_code)]

use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

/// Permit failures.
#[derive(Debug)]
pub enum PermitError {
    /// The run was cancelled; no new admission happens.
    RunCancelled,
    /// The journal writer is poisoned; no new admission happens.
    WriterUnhealthy,
    /// The configured limit is zero.
    Limit { reason: String },
}

impl fmt::Display for PermitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunCancelled => {
                write!(formatter, "no new admission: the run is cancelled")
            }
            Self::WriterUnhealthy => {
                write!(
                    formatter,
                    "no new admission: the journal writer is unhealthy"
                )
            }
            Self::Limit { reason } => write!(formatter, "permit limit failure: {reason}"),
        }
    }
}
impl Error for PermitError {}

/// The shared ledger state behind the permit handle.
#[derive(Debug, Default)]
struct LedgerState {
    active: BTreeSet<String>,
    queue: VecDeque<String>,
    cancelled: bool,
    writer_healthy: bool,
}

/// One granted permit; dropping it returns the slot to the ledger.
#[derive(Debug)]
pub struct Permit {
    pub repository: String,
    state: Arc<Mutex<LedgerState>>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.state
            .lock()
            .expect("permit ledger")
            .active
            .remove(&self.repository);
    }
}

/// The bounded permit ledger for one run.
#[derive(Clone, Debug)]
pub struct FleetPermits {
    limit: usize,
    state: Arc<Mutex<LedgerState>>,
}

impl FleetPermits {
    /// A zero limit is refused here: the zero/invalid case is handled
    /// before admission.
    pub fn new(limit: usize) -> Result<Self, PermitError> {
        if limit == 0 {
            return Err(PermitError::Limit {
                reason: "the active-repository limit must be greater than zero".to_owned(),
            });
        }
        Ok(Self {
            limit,
            state: Arc::new(Mutex::new(LedgerState {
                writer_healthy: true,
                ..LedgerState::default()
            })),
        })
    }

    /// Enqueue a ready repository (deterministic FIFO).
    pub fn enqueue(&self, repository: impl Into<String>) {
        self.state
            .lock()
            .expect("permit ledger")
            .queue
            .push_back(repository.into());
    }

    /// Grant the next queued permit while the run admits.  Cancellation and
    /// writer failure stop new admission; the active count never exceeds
    /// the limit.
    pub fn grant_next(&self) -> Result<Option<Permit>, PermitError> {
        let mut state = self.state.lock().expect("permit ledger");
        if state.cancelled {
            return Err(PermitError::RunCancelled);
        }
        if !state.writer_healthy {
            return Err(PermitError::WriterUnhealthy);
        }
        if state.active.len() >= self.limit {
            return Ok(None);
        }
        let Some(repository) = state.queue.pop_front() else {
            return Ok(None);
        };
        state.active.insert(repository.clone());
        Ok(Some(Permit {
            repository,
            state: Arc::clone(&self.state),
        }))
    }

    /// The active count (never above the limit).
    pub fn active(&self) -> usize {
        self.state.lock().expect("permit ledger").active.len()
    }

    /// The queue length (admission order affects performance only).
    pub fn queued(&self) -> usize {
        self.state.lock().expect("permit ledger").queue.len()
    }

    /// Mark the run cancelled: new admission stops.
    pub fn cancel(&self) {
        self.state.lock().expect("permit ledger").cancelled = true;
    }

    /// Mark the journal writer unhealthy: new admission stops.
    pub fn mark_writer_unhealthy(&self) {
        self.state.lock().expect("permit ledger").writer_healthy = false;
    }
}

#[cfg(test)]
mod fleet_permits_tests;
