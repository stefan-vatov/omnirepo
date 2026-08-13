//! Durable run summary folding.
//!
//! Every selected repository reaches exactly one terminal outcome
//! (success, failure, cancelled); the summary status derives from those
//! outcomes and the record state.  No failure is hidden or duplicated;
//! evidence references stay bounded and redacted.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The summary wire schema.
pub const SUMMARY_SCHEMA: &str = "omnirepo.run-summary.v1";

/// One repository's terminal outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepoOutcome {
    Success,
    Failure { reason: String },
    Cancelled,
}

/// The derived run status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryStatus {
    Success,
    Failed,
    Cancelled,
    Incomplete,
}

/// One repository entry in the summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoEntry {
    pub repository: String,
    pub outcome: RepoOutcome,
    /// A bounded, redacted evidence reference (never raw output).
    pub evidence: String,
}

/// The durable run summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSummary {
    pub schema: String,
    pub run_id: String,
    pub status: SummaryStatus,
    pub repositories: Vec<RepoEntry>,
}

/// Summary failures.
#[derive(Debug)]
pub enum SummaryError {
    DuplicateOutcome { repository: String },
    Empty,
}

impl fmt::Display for SummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOutcome { repository } => {
                write!(
                    formatter,
                    "repository {repository} has more than one terminal outcome"
                )
            }
            Self::Empty => write!(formatter, "the run has no repositories"),
        }
    }
}
impl Error for SummaryError {}

/// Fold the terminal outcomes into the durable summary.  Every repository
/// must appear exactly once; the status derives from the outcomes and the
/// record state (`record_terminal` is true when the journal record is
/// terminal).
pub fn fold_summary(
    run_id: impl Into<String>,
    outcomes: Vec<(String, RepoOutcome, String)>,
    record_terminal: bool,
) -> Result<RunSummary, SummaryError> {
    if outcomes.is_empty() {
        return Err(SummaryError::Empty);
    }
    let mut repositories = Vec::with_capacity(outcomes.len());
    let mut seen = Vec::with_capacity(outcomes.len());
    for (repository, outcome, evidence) in outcomes {
        if seen.contains(&repository) {
            return Err(SummaryError::DuplicateOutcome { repository });
        }
        seen.push(repository.clone());
        repositories.push(RepoEntry {
            repository,
            outcome,
            evidence,
        });
    }
    let mut failed = false;
    let mut cancelled = false;
    let mut any = false;
    for entry in &repositories {
        any = true;
        match &entry.outcome {
            RepoOutcome::Success => {}
            RepoOutcome::Failure { .. } => failed = true,
            RepoOutcome::Cancelled => cancelled = true,
        }
    }
    let status = if !any || !record_terminal {
        SummaryStatus::Incomplete
    } else if failed {
        SummaryStatus::Failed
    } else if cancelled {
        SummaryStatus::Cancelled
    } else {
        SummaryStatus::Success
    };
    Ok(RunSummary {
        schema: SUMMARY_SCHEMA.to_owned(),
        run_id: run_id.into(),
        status,
        repositories,
    })
}

/// Deterministic serialization of the summary.
pub fn render_summary(summary: &RunSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{SUMMARY_SCHEMA} run={} status={:?}\n",
        summary.run_id, summary.status
    ));
    for entry in &summary.repositories {
        let label = match &entry.outcome {
            RepoOutcome::Success => "success",
            RepoOutcome::Failure { .. } => "failure",
            RepoOutcome::Cancelled => "cancelled",
        };
        out.push_str(&format!(
            "repo={} outcome={} evidence={}\n",
            entry.repository, label, entry.evidence
        ));
    }
    out
}

#[cfg(test)]
mod run_summary_tests;
