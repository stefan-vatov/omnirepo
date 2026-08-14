//! Finalize the run: summary, terminal record, projection, and exit.
//!
//! The fleet response folds into the durable run summary — delivered
//! members succeed, failed and skipped members fail with their typed
//! reasons, and affected repositories (invalid plans) fold in as
//! failures.  The record finalizes with the derived terminal outcome
//! and the projection follows the quiet-human contract; the exit class
//! is exact (0/3/4).

#![allow(dead_code)]

#[cfg(test)]
mod fleet_finalize_tests;

use crate::lifecycle::event::{JournalEvent, Outcome};
use crate::lifecycle::exit_status::{ExitClass, classify_summary};
use crate::lifecycle::fleet_app::FleetResponse;
use crate::lifecycle::fleet_collector::MemberResult;
use crate::lifecycle::journal::JournalHandle;
use crate::lifecycle::run_summary::{RepoOutcome, RunSummary, fold_summary};
use crate::lifecycle::terminal_projection::render_human;

/// The finalization outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizeOutcome {
    pub summary: RunSummary,
    pub exit_class: ExitClass,
    pub projection: String,
}

/// Finalize the fleet run: fold the response into the summary, write
/// the terminal record, project, and derive the exact exit class.
pub fn finalize_fleet_run(
    journal: &JournalHandle,
    run_id: &str,
    response: &FleetResponse,
    affected: &[String],
) -> Result<FinalizeOutcome, String> {
    let mut outcomes = Vec::new();
    for result in &response.results {
        match result {
            MemberResult::Delivered { repository, oid } => {
                outcomes.push((
                    repository.clone(),
                    RepoOutcome::Success,
                    format!("commit/{oid}"),
                ));
            }
            MemberResult::Failed { repository, reason } => {
                outcomes.push((
                    repository.clone(),
                    RepoOutcome::Failure {
                        reason: reason.clone(),
                    },
                    "process/initial-pass".to_owned(),
                ));
            }
            MemberResult::Skipped { repository, reason } => {
                outcomes.push((
                    repository.clone(),
                    RepoOutcome::Failure {
                        reason: reason.clone(),
                    },
                    "process/preflight".to_owned(),
                ));
            }
        }
    }
    for entry in affected {
        let repository = entry.split(':').next().unwrap_or(entry).to_owned();
        outcomes.push((
            repository,
            RepoOutcome::Failure {
                reason: entry.clone(),
            },
            "process/plan".to_owned(),
        ));
    }
    let summary = if outcomes.is_empty() {
        // The empty fleet is a success: no repositories, quiet.
        RunSummary {
            schema: "omnirepo.summary.v1".to_owned(),
            run_id: run_id.to_owned(),
            status: crate::lifecycle::run_summary::SummaryStatus::Success,
            repositories: Vec::new(),
        }
    } else {
        fold_summary(run_id, outcomes, true).map_err(|error| error.to_string())?
    };
    let exit_class = classify_summary(&summary, true);
    let terminal = match exit_class {
        ExitClass::Success => Outcome::Success,
        _ => Outcome::Failed,
    };
    journal
        .submit(JournalEvent::Terminal {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            outcome: terminal,
        })
        .map_err(|error| error.to_string())?;
    let projection = render_human(
        &summary,
        true,
        match exit_class {
            ExitClass::Success => Some("sync complete"),
            _ => None,
        },
    );
    Ok(FinalizeOutcome {
        summary,
        exit_class,
        projection,
    })
}
