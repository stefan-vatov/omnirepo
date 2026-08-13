//! Quiet human and optional machine terminal projections.
//!
//! Success emits the decided zero or one concise line; failure names every
//! affected repository and stage plus a safe record reference; a
//! record-unavailable case has a truthful alternative.  The optional
//! machine mode is JSON lines with no human contamination.

#![allow(dead_code)]

use super::run_summary::{RepoOutcome, RunSummary, SummaryStatus};

/// The machine projection schema.
pub const PROJECTION_SCHEMA: &str = "omnirepo.terminal-projection.v1";

/// Render the human terminal projection.
///
/// Success: zero or one concise line (decided by the caller via
/// `success_line`).  Failure: one line naming every affected repository
/// and stage, plus the record reference.  Record unavailable: the
/// truthful alternative.
pub fn render_human(
    summary: &RunSummary,
    record_available: bool,
    success_line: Option<&str>,
) -> String {
    match summary.status {
        SummaryStatus::Success => match success_line {
            Some(line) => format!("{line}\n"),
            None => String::new(),
        },
        SummaryStatus::Incomplete => {
            if record_available {
                format!("sync incomplete: record {}\n", summary.run_id)
            } else {
                "sync incomplete: the run record is not available\n".to_owned()
            }
        }
        SummaryStatus::Cancelled => {
            if record_available {
                format!("sync cancelled: record {}\n", summary.run_id)
            } else {
                "sync cancelled: the run record is not available\n".to_owned()
            }
        }
        SummaryStatus::Failed => {
            let affected = summary
                .repositories
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    RepoOutcome::Failure { .. } => Some(entry.repository.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(", ");
            if record_available {
                format!("sync failed: {affected}; see record {}\n", summary.run_id)
            } else {
                format!("sync failed: {affected}; the run record is not available\n")
            }
        }
    }
}

/// Render the machine projection: JSON lines, no human contamination.
pub fn render_machine(summary: &RunSummary, record_available: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"schema\":\"{PROJECTION_SCHEMA}\",\"run\":\"{}\",\"status\":\"{}\",\"record\":\"{}\"}}\n",
        summary.run_id,
        status_label(summary.status),
        if record_available { "available" } else { "unavailable" }
    ));
    for entry in &summary.repositories {
        let outcome = match &entry.outcome {
            RepoOutcome::Success => "success",
            RepoOutcome::Failure { .. } => "failure",
            RepoOutcome::Cancelled => "cancelled",
        };
        out.push_str(&format!(
            "{{\"repo\":\"{}\",\"outcome\":\"{outcome}\",\"evidence\":\"{}\"}}\n",
            entry.repository, entry.evidence
        ));
    }
    out
}

fn status_label(status: SummaryStatus) -> &'static str {
    match status {
        SummaryStatus::Success => "success",
        SummaryStatus::Failed => "failed",
        SummaryStatus::Cancelled => "cancelled",
        SummaryStatus::Incomplete => "incomplete",
    }
}

#[cfg(test)]
mod terminal_projection_tests;
