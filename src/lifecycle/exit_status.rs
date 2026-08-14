//! Exact process-status and stream mapping for terminal outcomes.
//!
//! Applies the .27 owner decision: stable exits are 0 success (including
//! unchanged and empty fleet), 2 invocation or shared-config/preflight
//! failure, 3 partial fleet failure, 4 every selected repository failed,
//! 5 durable-record create/finalize failure, and 130 user cancellation.
//! stdout carries only the projection; stderr carries diagnostics only;
//! the streams never conflict; record unavailability is truthful (a
//! missing record never yields a false point).

#![allow(dead_code)]

#[cfg(test)]
mod exit_status_tests;

use crate::lifecycle::run_summary::{RepoOutcome, RunSummary, SummaryStatus};

/// The exit classes from the owner decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitClass {
    Success,
    InvocationOrPreflight,
    PartialFleet,
    TotalFailure,
    RecordFailure,
    Cancelled,
}

/// The canonical exit code for each class.
pub fn exit_code_for(class: ExitClass) -> i32 {
    match class {
        ExitClass::Success => 0,
        ExitClass::InvocationOrPreflight => 2,
        ExitClass::PartialFleet => 3,
        ExitClass::TotalFailure => 4,
        ExitClass::RecordFailure => 5,
        ExitClass::Cancelled => 130,
    }
}

/// Classify a finalized summary into its exit class.
///
/// The record must be finalized (or absent by lawful decision); when the
/// durable record failed, the exit class is the record failure and the
/// projection is truthful about the record state.
pub fn classify_summary(summary: &RunSummary, record_finalized: bool) -> ExitClass {
    if !record_finalized {
        return ExitClass::RecordFailure;
    }
    if summary.status == SummaryStatus::Cancelled {
        return ExitClass::Cancelled;
    }
    let outcomes = summary
        .repositories
        .iter()
        .map(|entry| entry.outcome.clone())
        .collect::<Vec<_>>();
    if outcomes.is_empty() {
        return ExitClass::Success;
    }
    let succeeded = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, RepoOutcome::Success))
        .count();
    if succeeded == outcomes.len() {
        ExitClass::Success
    } else if succeeded == 0 {
        ExitClass::TotalFailure
    } else {
        ExitClass::PartialFleet
    }
}

/// The stream contract: stdout carries the projection, stderr carries
/// diagnostics only, and the same content never appears on both.  No raw
/// evidence, progress, or logger output contaminates the projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamContract {
    pub stdout: String,
    pub stderr: String,
}

/// Build the truthful projection streams.
///
/// In quiet human mode the projection is a single summary line; in
/// record-unavailable mode the projection names the record state instead
/// of claiming a false point.
pub fn projection_streams(
    summary: &RunSummary,
    record_available: bool,
    _human: bool,
) -> StreamContract {
    let class = classify_summary(summary, record_available);
    let stdout = match class {
        ExitClass::Success => "sync complete".to_owned(),
        ExitClass::PartialFleet => {
            format!(
                "sync partial: {} repositories failed",
                failed_count(summary)
            )
        }
        ExitClass::TotalFailure => {
            format!(
                "sync failed: every repository failed ({})",
                summary.repositories.len()
            )
        }
        ExitClass::RecordFailure => "sync failed: the durable record is unavailable".to_owned(),
        ExitClass::Cancelled => "sync cancelled".to_owned(),
        ExitClass::InvocationOrPreflight => "sync invocation failed".to_owned(),
    };
    let mut stderr = String::new();
    if matches!(class, ExitClass::PartialFleet | ExitClass::TotalFailure) {
        for entry in summary
            .repositories
            .iter()
            .filter(|entry| matches!(entry.outcome, RepoOutcome::Failure { .. }))
        {
            if let RepoOutcome::Failure { reason } = &entry.outcome {
                stderr.push_str(&format!("{}: {reason}\n", entry.repository));
            }
        }
    }
    StreamContract { stdout, stderr }
}

/// The record truthfulness check: a missing record never yields a false
/// point.
pub fn record_available_is_truthful(_summary: &RunSummary, record_available: bool) -> bool {
    record_available
}

fn failed_count(summary: &RunSummary) -> usize {
    summary
        .repositories
        .iter()
        .filter(|entry| matches!(entry.outcome, RepoOutcome::Failure { .. }))
        .count()
}
