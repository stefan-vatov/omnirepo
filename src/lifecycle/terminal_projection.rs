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

/// Sanitize an identifier for projection: strip ANSI escape sequences and
/// every control character (including newlines), so no injection or
/// interleaving is possible.
pub fn sanitize_id(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_escape = false;
    for character in input.chars() {
        if in_escape {
            if ('@'..='~').contains(&character) {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        let code = character as u32;
        if code < 0x20 || code == 0x7f {
            continue;
        }
        output.push(character);
    }
    output
}

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
                    RepoOutcome::Failure { .. } => Some(sanitize_id(&entry.repository)),
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
        let repository = sanitize_id(&entry.repository);
        let evidence = sanitize_id(&entry.evidence);
        out.push_str(&format!(
            "{{\"repo\":\"{repository}\",\"outcome\":\"{outcome}\",\"evidence\":\"{evidence}\"}}\n"
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

#[cfg(test)]
mod summary_fixture_tests;
