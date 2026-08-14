//! Repair input freezing and durable one-attempt reservation.
//!
//! A repairable failure freezes its repair inputs (deduplicated,
//! non-empty) and durably reserves exactly one attempt per repository per
//! run through the journal.  A second reservation is refused; empty inputs
//! fail typed.

#![allow(dead_code)]

use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent};

#[cfg(test)]
mod repair_reserve_tests;
use crate::lifecycle::journal::{JournalError, JournalHandle};
use std::{
    error::Error,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

/// The reservation outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReserveOutcome {
    /// Exactly one attempt was reserved and journaled.
    Reserved(RepairReservation),
}

/// One durable reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairReservation {
    pub reservation_id: String,
    pub repository: String,
    pub attempt: u8,
    pub frozen_inputs: Vec<String>,
    pub journaled: bool,
}

/// Reservation failures.
#[derive(Debug)]
pub enum ReserveError {
    NoFrozenInputs { repository: String },
    AlreadyReserved { repository: String },
    AttemptsExhausted { repository: String },
    Journal(JournalError),
}

impl fmt::Display for ReserveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrozenInputs { repository } => {
                write!(
                    formatter,
                    "repository {repository} has no frozen repair inputs"
                )
            }
            Self::AlreadyReserved { repository } => write!(
                formatter,
                "repository {repository} already has a reserved repair attempt in this run"
            ),
            Self::AttemptsExhausted { repository } => {
                write!(
                    formatter,
                    "repository {repository} exhausted its repair attempts"
                )
            }
            Self::Journal(error) => {
                write!(formatter, "repair reservation journal failure: {error}")
            }
        }
    }
}
impl Error for ReserveError {}

/// Reserve exactly one repair attempt per repository per run.  The inputs
/// are frozen (deduplicated in declared order); the reservation is
/// journaled durably before it is returned.
pub fn reserve_repair_attempt(
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    inputs: &[String],
    max_attempts: u8,
    record: &str,
) -> Result<ReserveOutcome, ReserveError> {
    detect_duplicate_reservation(record, repository)?;
    let frozen: Vec<String> = {
        let mut seen = Vec::new();
        for input in inputs {
            if !seen.contains(input) {
                seen.push(input.clone());
            }
        }
        seen
    };
    if frozen.is_empty() {
        return Err(ReserveError::NoFrozenInputs {
            repository: repository.to_owned(),
        });
    }
    if max_attempts == 0 {
        return Err(ReserveError::AttemptsExhausted {
            repository: repository.to_owned(),
        });
    }
    // The journal is the durable reservation: the evidence event is
    // committed before the reservation is returned, and a duplicate
    // reservation is detected through the reservation marker.
    let marker = format!("repair/{repository}/attempt/1");
    let reservation_id = format!(
        "repair-{run_id}-{repository}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    );
    let evidence = EvidenceRef::new(EvidenceKind::Process, marker, 1)
        .map_err(|error| ReserveError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: Some(repository.to_owned()),
            evidence,
            stage: Some("repair-reserve"),
        })
        .map_err(ReserveError::Journal)?;
    Ok(ReserveOutcome::Reserved(RepairReservation {
        reservation_id,
        repository: repository.to_owned(),
        attempt: 1,
        frozen_inputs: frozen,
        journaled: true,
    }))
}

/// Detect a duplicate reservation from the run record.
pub fn detect_duplicate_reservation(record: &str, repository: &str) -> Result<(), ReserveError> {
    let marker = format!("\"path\":\"repair/{repository}/attempt/1\"");
    if record.contains(&marker) {
        return Err(ReserveError::AlreadyReserved {
            repository: repository.to_owned(),
        });
    }
    Ok(())
}
