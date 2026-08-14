//! Wire cancellation into the CLI.
//!
//! A cancellation stops the fleet run: every selected repository —
//! queued and in-flight — receives a terminal cancellation
//! classification, the durable record finalizes as cancelled when
//! possible, and the run exits with the cancellation class (130).
//! Completed Git effects are reconciled and retained, never rolled back.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_cancel_tests;

use crate::lifecycle::cancellation::cancel_run;
use crate::lifecycle::exit_status::ExitClass;
use crate::lifecycle::journal::JournalHandle;

/// The cancellation outcome for the exit mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelOutcome {
    pub exit_class: ExitClass,
}

/// Cancel the fleet run: classify every selected repository and
/// finalize the record when possible.
pub fn cancel_fleet_run(
    journal: &JournalHandle,
    run_id: &str,
    fleet: &[String],
) -> Result<CancelOutcome, String> {
    if fleet.is_empty() {
        // A run that never admitted a repository still finalizes as
        // cancelled.
        crate::lifecycle::cancellation::terminalize_not_started(journal, run_id)
            .map_err(|error| error.to_string())?;
    } else {
        cancel_run(journal, run_id, fleet).map_err(|error| error.to_string())?;
    }
    Ok(CancelOutcome {
        exit_class: ExitClass::Cancelled,
    })
}
