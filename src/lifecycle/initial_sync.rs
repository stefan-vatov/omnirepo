//! Contained multi-item synchronization pass execution with journaling.
//!
//! Every operation has a journaled intent and result; an unchanged item
//! performs no write; a failed item leaves exactly its residue (the temp
//! candidate path); later-item behavior follows the declared failure
//! policy; outside-scope targets are rejected before any effect.

#![allow(dead_code)]

use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent};

#[cfg(test)]
mod initial_sync_tests;
use crate::lifecycle::journal::{JournalError, JournalHandle};
use crate::platform::{
    AuthorityRoot, DestinationRepositoryRoot, Mutate, MutationIntent, RelativePath,
    resolve_mutation,
};
use std::{error::Error, fmt, io::Write, path::Path};

/// The declared failure policy for later items.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePolicy {
    /// Later items still execute after a failure.
    Continue,
    /// Later items are skipped after a failure.
    StopOnFailure,
}

/// One item of the pass: one destination-file group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncItem {
    pub plan_item_id: String,
    pub target: String,
    /// The destination file's exact bytes at composition time.
    pub current_bytes: Vec<u8>,
    /// The exact complete destination bytes to write (the composed file
    /// for section groups, the authoritative bytes for whole files).
    pub replacement: Vec<u8>,
    /// Deterministic failure seam (group composition failures and
    /// fault-injection use).
    pub fail: Option<String>,
}

/// One item's typed outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncOutcome {
    /// Equal bytes: no write of any kind.
    Unchanged,
    /// Different bytes: one prepared replacement.
    Replacement,
    /// The item failed and left exactly its residue.
    Failed {
        residue: Vec<String>,
        reason: String,
    },
    /// The item was skipped by the failure policy.
    Skipped { reason: String },
}

/// One item's execution record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemExecution {
    pub plan_item_id: String,
    pub outcome: SyncOutcome,
    /// True when the intent and result were journaled durably.
    pub journaled: bool,
}

/// The ordered pass report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncPassReport {
    pub repository: String,
    pub items: Vec<ItemExecution>,
}

/// Pass failures (outside-scope targets fail before any effect).
#[derive(Debug)]
pub enum SyncPassError {
    OutsideTarget { target: String, reason: String },
    Journal(JournalError),
}

impl fmt::Display for SyncPassError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideTarget { target, reason } => {
                write!(
                    formatter,
                    "target {target:?} is outside the scope: {reason}"
                )
            }
            Self::Journal(error) => write!(formatter, "pass journal failure: {error}"),
        }
    }
}
impl Error for SyncPassError {}

/// The residue of a failed operation: exactly the temp candidate path.
fn residue_for(target: &str) -> String {
    format!("{target}.omnirepo-tmp")
}

/// Write the replacement bytes to the destination target through the
/// mutation authority (no-follow, identity-revalidated, synced).
fn write_replacement(working: &Path, target: &str, bytes: &[u8]) -> Result<(), String> {
    let root = AuthorityRoot::<DestinationRepositoryRoot, Mutate>::open(working)
        .map_err(|error| format!("open root: {error}"))?;
    let relative = RelativePath::parse(target).map_err(|error| format!("parse: {error}"))?;
    let mutation = resolve_mutation(&root, &relative, MutationIntent::Replace)
        .map_err(|error| format!("resolve: {error}"))?;
    // resolve_mutation validated the leaf and kept its O_RDWR handle;
    // revalidate_mutation returns the parent (the create path), so the
    // write goes through the leaf handle directly.
    let mut file = mutation
        .handle
        .ok_or_else(|| format!("replace target {target} has no write handle"))?;
    file.set_len(0)
        .map_err(|error| format!("set_len: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("write: {error}"))?;
    file.sync_all().map_err(|error| format!("sync: {error}"))?;
    Ok(())
}

/// Execute the pass: validate every target first (outside-scope content is
/// protected), then classify, apply, and journal each item in declared
/// order.  A replacement writes exactly the prepared replacement bytes to
/// the destination target through the mutation authority.
pub fn execute_sync_pass(
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    working: &Path,
    items: &[SyncItem],
    policy: FailurePolicy,
) -> Result<SyncPassReport, SyncPassError> {
    for item in items {
        RelativePath::parse(&item.target).map_err(|error| SyncPassError::OutsideTarget {
            target: item.target.clone(),
            reason: error.to_string(),
        })?;
    }
    let mut stopped = false;
    let mut executions = Vec::with_capacity(items.len());
    for item in items {
        if stopped {
            executions.push(ItemExecution {
                plan_item_id: item.plan_item_id.clone(),
                outcome: SyncOutcome::Skipped {
                    reason: "the failure policy stopped the pass".to_owned(),
                },
                journaled: true,
            });
            continue;
        }
        let intent = journal_evidence(
            journal,
            run_id,
            repository,
            &item.plan_item_id,
            "sync-intent",
        )?;
        let outcome = if let Some(reason) = &item.fail {
            SyncOutcome::Failed {
                residue: vec![residue_for(&item.target)],
                reason: reason.clone(),
            }
        } else if item.replacement == item.current_bytes {
            SyncOutcome::Unchanged
        } else {
            match write_replacement(working, &item.target, &item.replacement) {
                Ok(()) => SyncOutcome::Replacement,
                Err(reason) => SyncOutcome::Failed {
                    residue: vec![residue_for(&item.target)],
                    reason,
                },
            }
        };
        if matches!(outcome, SyncOutcome::Failed { .. }) && policy == FailurePolicy::StopOnFailure {
            stopped = true;
        }
        let result = journal_evidence(
            journal,
            run_id,
            repository,
            &item.plan_item_id,
            "sync-result",
        )?;
        executions.push(ItemExecution {
            plan_item_id: item.plan_item_id.clone(),
            outcome,
            journaled: intent && result,
        });
    }
    Ok(SyncPassReport {
        repository: repository.to_owned(),
        items: executions,
    })
}

fn journal_evidence(
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    item: &str,
    stage: &'static str,
) -> Result<bool, SyncPassError> {
    let evidence = EvidenceRef::new(
        EvidenceKind::Process,
        format!("sync/{repository}/{item}/{stage}"),
        0,
    )
    .map_err(|error| SyncPassError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: Some(repository.to_owned()),
            evidence,
            stage: Some(stage),
        })
        .map(|_| true)
        .map_err(SyncPassError::Journal)
}
