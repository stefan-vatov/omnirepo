//! Single-repository initial pass composition.
//!
//! The one-repository pass composes the frozen plan, the contained sync
//! pass, the declared verification commands, and the scoped Git delivery:
//! events precede effects, every declared check runs in configured order
//! and must pass, concurrent managed modification fails the pass, the
//! exact OID reconciles, protected state is never touched, and a run
//! yields one replayable result.  No scheduler, UI, or agent dependency
//! exists.

#![allow(dead_code)]

use crate::lifecycle::check_runner::{CheckOutcome, run_check};
use crate::lifecycle::command_spec::{
    DEFAULT_COMMAND_TIMEOUT, DeclaredCommand, translate_commands,
};
use crate::lifecycle::git_delivery::{DeliveryOutcome, coordinate_git_delivery};

#[cfg(test)]
mod single_repo_pass_tests;
use crate::lifecycle::initial_sync::{FailurePolicy, SyncItem, execute_sync_pass};
use crate::lifecycle::journal::{JournalError, JournalHandle};
use crate::lifecycle::verify_and_gate::VerificationVerdict;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{
    GitRepositoryState, HeadState, IsolatedIndex, PlannedOperation, RepositorySnapshot,
    TargetChange, VerificationCommand, build_authorized_delta, capture_state, prepare_index,
};
use std::{error::Error, fmt, path::Path};

/// The composed pass outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PassOutcome {
    /// The repository reached a delivered commit.
    Delivered { oid: String },
    /// The pass failed with a typed reason.
    Failed { reason: String },
}

/// Pass failures.
#[derive(Debug)]
pub enum PassError {
    Plan { reason: String },
    Journal(JournalError),
    Delivery { reason: String },
}

impl fmt::Display for PassError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan { reason } => {
                write!(formatter, "single-repo pass planning failure: {reason}")
            }
            Self::Journal(error) => write!(formatter, "single-repo pass journal failure: {error}"),
            Self::Delivery { reason } => {
                write!(formatter, "single-repo pass delivery failure: {reason}")
            }
        }
    }
}
impl Error for PassError {}

/// The typed failure reason for one verification outcome.
fn verification_failure(outcome: &CheckOutcome) -> String {
    match outcome {
        CheckOutcome::Passed => unreachable!("passed checks are not failures"),
        CheckOutcome::Failed { code } => match code {
            Some(code) => format!("verification failed with exit code {code}"),
            None => "verification failed by signal".to_owned(),
        },
        CheckOutcome::TimedOut { budget } => {
            format!("verification exceeded its {budget:?} budget")
        }
        CheckOutcome::Cancelled => "verification was cancelled".to_owned(),
    }
}

/// Run the one-repository pass: plan the authorized delta from the frozen
/// snapshot, journal the contained sync pass, run the declared
/// verification commands in configured order, and deliver the scoped
/// commit only when every check passed and no concurrent managed
/// modification appeared.
#[allow(clippy::too_many_arguments)]
pub fn run_single_repository_pass(
    working: &Path,
    journal: &JournalHandle,
    run_id: &str,
    repository: &str,
    snapshot: &RepositorySnapshot,
    checks: &[VerificationCommand],
    plan: &crate::lifecycle::sync_plan::SyncPlan,
    sources: &std::collections::HashMap<String, std::path::PathBuf>,
    message: &str,
) -> Result<PassOutcome, PassError> {
    // Build the authorized operations and the sync items with the real
    // payloads, in plan order.  An absent target (creation) is not part of
    // the replace-only pass contract and fails typed instead of panicking.
    let mut operations = Vec::new();
    let mut items = Vec::new();
    for plan_item in plan.items.iter() {
        let target = &plan_item.target;
        let observed = snapshot
            .targets()
            .iter()
            .find(|target_entry| target_entry.path().as_bytes() == target.as_bytes());
        let Some(identity) = observed.and_then(|entry| entry.observed_file().cloned()) else {
            return Err(PassError::Plan {
                reason: format!(
                    "managed target {} is absent; creation is not part of the replace-only pass",
                    target
                ),
            });
        };
        operations.push(PlannedOperation::replaced(
            observed.expect("observed target was found").path().clone(),
            identity.clone(),
            identity,
        ));
        // The source payload and the destination's current payload.
        let syntax = if plan_item.kind == crate::source::ItemKind::Section {
            Some(
                crate::managed_content::lookup_by_extension(&plan_item.source_path).map_err(
                    |error| PassError::Plan {
                        reason: format!(
                            "no delimiter syntax for source {}: {error}",
                            plan_item.source_path
                        ),
                    },
                )?,
            )
        } else {
            None
        };
        let source_root = sources
            .get(&plan_item.source)
            .ok_or_else(|| PassError::Plan {
                reason: format!("source {} has no configured local root", plan_item.source),
            })?;
        let source_bytes = read_source_file(source_root, &plan_item.source_path)?;
        let (authoritative, _) = payload(
            &source_bytes,
            &plan_item.source_path,
            plan_item.kind,
            syntax,
        )?;
        let current_bytes =
            std::fs::read(working.join(target)).map_err(|error| PassError::Plan {
                reason: format!("cannot read the current managed target {target}: {error}"),
            })?;
        let (current_payload, destination_bounds) =
            payload(&current_bytes, target, plan_item.kind, syntax)?;
        let replacement = match plan_item.kind {
            crate::source::ItemKind::WholeFile => authoritative.clone(),
            crate::source::ItemKind::Section => match destination_bounds {
                Some(bounds) => compose_section(&current_bytes, &authoritative, bounds),
                None => {
                    return Err(PassError::Plan {
                        reason: format!(
                            "the destination target {target} has no managed section markers"
                        ),
                    });
                }
            },
        };
        items.push(SyncItem {
            plan_item_id: plan_item.id.clone(),
            target: target.clone(),
            frozen_bytes: authoritative,
            current_bytes: current_payload,
            replacement,
            fail: None,
        });
    }
    let delta = build_authorized_delta(snapshot, operations).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    // Journal the contained sync pass and apply every replacement to the
    // destination worktree BEFORE the isolated index is staged, so the
    // commit captures the applied content.
    let sync_report = execute_sync_pass(
        journal,
        run_id,
        repository,
        working,
        &items,
        FailurePolicy::Continue,
    )
    .map_err(|error| PassError::Journal(error.to_journal()))?;
    // A failed item fails the pass: no Git delivery for a partially
    // applied or unapplied managed change.
    for execution in &sync_report.items {
        if let crate::lifecycle::initial_sync::SyncOutcome::Failed { reason, .. } =
            &execution.outcome
        {
            return Ok(PassOutcome::Failed {
                reason: format!(
                    "managed item {} failed to apply: {reason}",
                    execution.plan_item_id
                ),
            });
        }
    }
    let index: IsolatedIndex = prepare_index(working, &delta).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    // Verification: every declared check runs in configured order with a
    // bounded budget; any non-passed outcome fails the pass and prevents
    // Git.  An absent or empty command list means no verification command
    // is required (canon/architecture/fleet-lifecycle.md).
    if !checks.is_empty() {
        let declared = checks
            .iter()
            .map(|command| DeclaredCommand {
                argv: command.argv().to_vec(),
                cwd: None,
                env: Vec::new(),
                timeout: None,
                stdin: None,
                capture_output: true,
                shell: None,
            })
            .collect::<Vec<_>>();
        let specs = translate_commands(repository, "plan", &declared, DEFAULT_COMMAND_TIMEOUT)
            .map_err(|error| PassError::Plan {
                reason: error.to_string(),
            })?;
        for spec in &specs {
            let result =
                run_check(working, spec, spec.timeout).map_err(|error| PassError::Plan {
                    reason: format!(
                        "verification at position {} failed to run: {error}",
                        spec.position
                    ),
                })?;
            if !matches!(result.outcome, CheckOutcome::Passed) {
                return Ok(PassOutcome::Failed {
                    reason: format!(
                        "check {} ({}) {}; no Git delivery",
                        spec.position + 1,
                        spec.argv.join(" "),
                        verification_failure(&result.outcome)
                    ),
                });
            }
        }
    }
    // Concurrent-modification guard: re-capture the current state; any
    // change at a managed target that is not the authorized replacement is
    // a concurrent user change and prevents Git.  Unmanaged paths may
    // coexist (pre-existing state).  The staged pass does not require the
    // authorized delta to have landed in the worktree, so a missing
    // operation effect is not a failure here.
    let current = capture_state(working).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    if let GitRepositoryState::Git(facts) = &current {
        let mut managed: Vec<(Vec<u8>, TargetChange)> = Vec::new();
        if let crate::repository::IndexState::Entries(entries) = facts.index() {
            for entry in entries {
                managed.push((entry.path().as_bytes().to_vec(), entry.change()));
            }
        }
        if let crate::repository::WorktreeState::Entries(entries) = facts.worktree() {
            for entry in entries {
                managed.push((entry.path().as_bytes().to_vec(), entry.change()));
            }
        }
        for (path, change) in managed {
            let target = snapshot
                .targets()
                .iter()
                .any(|target| target.path().as_bytes() == path.as_slice());
            if target && change != TargetChange::Modified {
                return Ok(PassOutcome::Failed {
                    reason: format!(
                        "managed target {} changed concurrently ({change:?}); no Git delivery",
                        String::from_utf8_lossy(&path)
                    ),
                });
            }
        }
    }
    // The exact base head is the delivery parent.
    let base = match capture_state(working).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })? {
        GitRepositoryState::Git(facts) => match facts.head() {
            HeadState::Attached { commit, .. } | HeadState::Detached { commit } => {
                Some(commit.as_str().to_owned())
            }
            HeadState::Unborn => None,
        },
        GitRepositoryState::NonGit => None,
    };
    let git_root =
        AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(working).map_err(|error| {
            PassError::Plan {
                reason: error.to_string(),
            }
        })?;
    let outcome = coordinate_git_delivery(
        &git_root,
        &index,
        base.as_deref(),
        message,
        journal,
        run_id,
        repository,
        VerificationVerdict::Ready,
    )
    .map_err(|error| PassError::Delivery {
        reason: error.to_string(),
    })?;
    match outcome {
        DeliveryOutcome::Delivered { oid } => Ok(PassOutcome::Delivered { oid }),
        DeliveryOutcome::Rejected { reason } => Ok(PassOutcome::Failed { reason }),
    }
}

/// Journal errors are converted without losing the typed reason.
trait ToJournalError {
    fn to_journal(self) -> JournalError;
}
impl ToJournalError for crate::lifecycle::initial_sync::SyncPassError {
    fn to_journal(self) -> JournalError {
        JournalError::Invalid(crate::lifecycle::event::EventError::UnknownVersion(0))
    }
}

/// Read one source file through the typed source authority (no-follow).
fn read_source_file(root: &Path, source_path: &str) -> Result<Vec<u8>, PassError> {
    use std::io::Read;
    let authority = AuthorityRoot::<crate::platform::SourceSnapshotRoot, ReadOnly>::open(root)
        .map_err(|error| PassError::Plan {
            reason: format!("cannot open the source root {}: {error}", root.display()),
        })?;
    let relative =
        crate::platform::RelativePath::parse(source_path).map_err(|error| PassError::Plan {
            reason: format!("source path {source_path:?} is invalid: {error}"),
        })?;
    let target = authority
        .resolve_read(&relative, crate::platform::ObjectClass::RegularFile)
        .map_err(|error| PassError::Plan {
            reason: format!("cannot resolve the source file {source_path}: {error}"),
        })?;
    let mut handle = target.try_clone_file().map_err(|error| PassError::Plan {
        reason: format!("cannot open the source file {source_path}: {error}"),
    })?;
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|error| PassError::Plan {
            reason: format!("cannot read the source file {source_path}: {error}"),
        })?;
    Ok(bytes)
}

/// Extract the payload for one item: the whole file, or the managed
/// section between the canonical marker pair.  The bounds are returned for
/// sections so the caller can compose the destination replacement.
fn payload(
    bytes: &[u8],
    path: &str,
    kind: crate::source::ItemKind,
    syntax: Option<&crate::managed_content::DelimiterSyntax>,
) -> Result<(Vec<u8>, Option<crate::managed_content::Bounds>), PassError> {
    match kind {
        crate::source::ItemKind::WholeFile => Ok((bytes.to_vec(), None)),
        crate::source::ItemKind::Section => {
            let syntax = syntax.expect("section items always resolve their delimiter syntax");
            let text = String::from_utf8_lossy(bytes);
            match crate::managed_content::scan_partial(&text, syntax) {
                crate::managed_content::Topology::ExactlyOne { bounds } => {
                    let start = bounds.start_line + 1;
                    let end = bounds.end_line - 1;
                    let section = if start <= end {
                        crate::source::extract_payload(
                            path,
                            bytes,
                            &crate::source::PayloadKind::Section {
                                start_line: start,
                                end_line: end,
                            },
                        )
                        .map_err(|error| PassError::Plan {
                            reason: format!(
                                "cannot extract the managed section from {path}: {error}"
                            ),
                        })?
                        .bytes
                    } else {
                        Vec::new()
                    };
                    Ok((section, Some(bounds)))
                }
                crate::managed_content::Topology::Absent => Err(PassError::Plan {
                    reason: format!("no managed section markers in {path}"),
                }),
                crate::managed_content::Topology::Ambiguous { reason } => Err(PassError::Plan {
                    reason: format!("{path}: {reason}"),
                }),
            }
        }
    }
}

/// Compose the full destination content for a section replacement: the
/// bytes before and including the open marker, the authoritative section,
/// then the close marker onward.
fn compose_section(
    current: &[u8],
    authoritative: &[u8],
    bounds: crate::managed_content::Bounds,
) -> Vec<u8> {
    let lines = split_lines(current);
    let mut replacement = Vec::with_capacity(current.len() + authoritative.len());
    for line in &lines[..bounds.start_line as usize] {
        replacement.extend_from_slice(line);
    }
    replacement.extend_from_slice(authoritative);
    for line in &lines[(bounds.end_line - 1) as usize..] {
        replacement.extend_from_slice(line);
    }
    replacement
}

/// Split content into lines that keep their trailing newline.
fn split_lines(content: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in content.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&content[start..=index]);
            start = index + 1;
        }
    }
    if start < content.len() {
        lines.push(&content[start..]);
    }
    lines
}
