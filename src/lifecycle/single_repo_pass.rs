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
    // A destination that is not itself a git repository cannot deliver:
    // fail typed before any effect, so no git command can walk out of the
    // destination root into an enclosing repository.
    if matches!(snapshot.facts().git(), GitRepositoryState::NonGit) {
        return Ok(PassOutcome::Failed {
            reason: "the destination is not a git repository; no synchronization or Git delivery"
                .to_owned(),
        });
    }
    // The plan owns the atomic per-file operation groups
    // (canon/architecture/fleet-lifecycle.md): all operations targeting
    // one destination file form one group, in plan order.  A group that
    // cannot compose (absent target, unknown destination format,
    // ambiguous topology, unreadable source) becomes a failed item: one
    // group's failure never prevents independent groups in the same
    // repository from being attempted.  An absent target is a lawful
    // creation case (canon/architecture/managed-content.md): the sync
    // pass creates it with mode 0644 and safe contained parents.
    let groups = plan.selected_target_groups();
    // One authority root per configured source, opened once for the pass.
    let mut source_roots = std::collections::HashMap::new();
    for (_, members) in &groups {
        for member in members {
            if source_roots.contains_key(&member.source) {
                continue;
            }
            let root_path = sources.get(&member.source).ok_or_else(|| PassError::Plan {
                reason: format!("source {} has no configured local root", member.source),
            })?;
            let root =
                AuthorityRoot::<crate::platform::SourceSnapshotRoot, ReadOnly>::open(root_path)
                    .map_err(|error| PassError::Plan {
                        reason: format!(
                            "cannot open the source root {}: {error}",
                            root_path.display()
                        ),
                    })?;
            source_roots.insert(member.source.clone(), root);
        }
    }
    let mut composed: Vec<ComposedGroup> = Vec::new();
    let mut items = Vec::new();
    for (target, members) in &groups {
        match compose_group(working, snapshot, &source_roots, target, members) {
            Ok(group) => {
                items.push(SyncItem {
                    plan_item_id: (*target).to_owned(),
                    target: (*target).to_owned(),
                    current_bytes: group.current_bytes.clone(),
                    replacement: group.replacement.clone(),
                    create: group.observed.is_none(),
                    fail: None,
                });
                composed.push(group);
            }
            Err(reason) => {
                // The group is failed, not the pass: peers still apply.
                items.push(SyncItem {
                    plan_item_id: (*target).to_owned(),
                    target: (*target).to_owned(),
                    current_bytes: Vec::new(),
                    replacement: Vec::new(),
                    create: false,
                    fail: Some(reason),
                });
            }
        }
    }
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
    // A failed group fails the pass outcome — no Git delivery for a
    // partially applied repository — but only after every independent
    // group had its attempt; successful group writes stay in the
    // worktree for repair (canon/architecture/fleet-lifecycle.md).
    for execution in &sync_report.items {
        if let crate::lifecycle::initial_sync::SyncOutcome::Failed { reason, .. } =
            &execution.outcome
        {
            return Ok(PassOutcome::Failed {
                reason: format!(
                    "managed group {} failed to apply: {reason}",
                    execution.plan_item_id
                ),
            });
        }
    }
    // Build the authorized operations after the pass so a created target
    // contributes its real published identity.
    let mut operations = Vec::new();
    let mut created_targets: Vec<Vec<u8>> = Vec::new();
    let observe_root =
        AuthorityRoot::<crate::platform::DestinationRepositoryRoot, ReadOnly>::open(working)
            .map_err(|error| PassError::Plan {
                reason: error.to_string(),
            })?;
    for group in &composed {
        match &group.observed {
            Some(identity) => operations.push(PlannedOperation::replaced(
                group.path.clone(),
                identity.clone(),
                identity.clone(),
            )),
            None => {
                let relative =
                    crate::platform::RelativePath::parse(&group.target).map_err(|error| {
                        PassError::Plan {
                            reason: error.to_string(),
                        }
                    })?;
                let after = crate::lifecycle::fleet_snapshot::observe_target_identity(
                    &observe_root,
                    working,
                    &group.target,
                    &relative,
                )
                .map_err(|reason| PassError::Plan { reason })?
                .ok_or_else(|| PassError::Plan {
                    reason: format!("created target {} is absent after the pass", group.target),
                })?;
                created_targets.push(group.target.as_bytes().to_vec());
                operations.push(PlannedOperation::added(group.path.clone(), after));
            }
        }
    }
    let delta = build_authorized_delta(snapshot, operations).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    let index: IsolatedIndex = prepare_index(working, &delta).map_err(|error| PassError::Plan {
        reason: error.to_string(),
    })?;
    let expected_modes = composed
        .iter()
        .map(|group| {
            read_managed_file(&observe_root, &group.target)
                .map(|(_, mode)| mode)
                .map_err(|reason| PassError::Plan { reason })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Verification: every declared check runs in configured order with a
    // bounded budget; any non-passed outcome fails the pass and prevents
    // Git.  An absent or empty command list means no verification command
    // is required (canon/architecture/fleet-lifecycle.md).
    let mut verification_failures = Vec::new();
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
            match run_check(working, spec, spec.timeout) {
                Ok(result) if !matches!(result.outcome, CheckOutcome::Passed) => {
                    verification_failures.push(format!(
                        "check {} ({}) {}; no Git delivery",
                        spec.position + 1,
                        spec.argv.join(" "),
                        verification_failure(&result.outcome)
                    ));
                }
                Err(error) => verification_failures.push(format!(
                    "check {} ({}) failed to run: {error}; no Git delivery",
                    spec.position + 1,
                    spec.argv.join(" ")
                )),
                Ok(_) => {}
            }
        }
    }
    // Verification checks are untrusted effects. Re-read every selected
    // managed target through the destination authority and restore the exact
    // authoritative bytes and mode when a check changed them. The pass then
    // fails without Git, while successful synchronization writes remain.
    let mut verifier_changes = Vec::new();
    let mut changed_bytes = false;
    let mut changed_metadata = false;
    for (group, expected_mode) in composed.iter().zip(expected_modes) {
        let (bytes_changed, metadata_changed) =
            match read_managed_file(&observe_root, &group.target) {
                Ok((bytes, mode)) => (bytes != group.replacement, mode != expected_mode),
                Err(_) => (true, true),
            };
        if !bytes_changed && !metadata_changed {
            continue;
        }
        changed_bytes |= bytes_changed;
        changed_metadata |= metadata_changed;
        let operation_id = format!("verification-restore-{run_id}");
        let restoration = crate::lifecycle::replace::replace_bytes_atomically_with_mode(
            working,
            &group.target,
            &operation_id,
            &group.replacement,
            expected_mode,
        );
        match restoration {
            Ok(()) => verifier_changes.push(format!("{} (restored)", group.target)),
            Err(error) => {
                verifier_changes.push(format!("{} (restoration failed: {error})", group.target))
            }
        }
    }
    if !verifier_changes.is_empty() {
        let mut changed = Vec::new();
        if changed_bytes {
            changed.push("managed bytes");
        }
        if changed_metadata {
            changed.push("managed metadata");
        }
        verification_failures.push(format!(
            "verification changed {} at {}; no Git delivery",
            changed.join(" and "),
            verifier_changes.join(", ")
        ));
    }
    if !verification_failures.is_empty() {
        return Ok(PassOutcome::Failed {
            reason: verification_failures.join("; "),
        });
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
            // The authorized effect is a modification for a replaced
            // target and an addition for a created target; anything else
            // at a managed path is a concurrent change.
            let authorized = if created_targets.iter().any(|created| created == &path) {
                matches!(change, TargetChange::Added | TargetChange::Untracked)
            } else {
                change == TargetChange::Modified
            };
            if target && !authorized {
                return Ok(PassOutcome::Failed {
                    reason: format!(
                        "managed target {} changed concurrently ({change:?}); no Git delivery",
                        String::from_utf8_lossy(&path)
                    ),
                });
            }
        }
    }
    // A candidate tree that is byte-identical to the frozen base still runs
    // its declared checks, but it creates no commit object. Return the base
    // as the stable delivered identity after the complete gate succeeds.
    if let Some(base) = snapshot.witnesses().base_head()
        && crate::repository::index_matches_parent(working, &index, base.as_str()).map_err(
            |error| PassError::Plan {
                reason: error.to_string(),
            },
        )?
    {
        journal
            .submit(crate::lifecycle::event::JournalEvent::RepositoryIntent {
                checkpoint: 0,
                run_id: run_id.to_owned(),
                repository_id: repository.to_owned(),
                operation: crate::lifecycle::event::Operation::Synchronize,
                attempt: 1,
            })
            .map_err(PassError::Journal)?;
        journal
            .submit(crate::lifecycle::event::JournalEvent::RepositoryResult {
                checkpoint: 0,
                run_id: run_id.to_owned(),
                repository_id: repository.to_owned(),
                operation: crate::lifecycle::event::Operation::Synchronize,
                attempt: 1,
                outcome: crate::lifecycle::event::Outcome::Success,
            })
            .map_err(PassError::Journal)?;
        return Ok(PassOutcome::Delivered {
            oid: base.as_str().to_owned(),
        });
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
fn read_source_file(
    authority: &AuthorityRoot<crate::platform::SourceSnapshotRoot, ReadOnly>,
    source_path: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let relative = crate::platform::RelativePath::parse(source_path)
        .map_err(|error| format!("source path {source_path:?} is invalid: {error}"))?;
    let target = authority
        .resolve_read(&relative, crate::platform::ObjectClass::RegularFile)
        .map_err(|error| format!("cannot resolve the source file {source_path}: {error}"))?;
    let mut handle = target
        .try_clone_file()
        .map_err(|error| format!("cannot open the source file {source_path}: {error}"))?;
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read the source file {source_path}: {error}"))?;
    Ok(bytes)
}

/// Read one managed destination file through the typed destination
/// authority (no-follow).
fn read_managed_file(
    authority: &AuthorityRoot<crate::platform::DestinationRepositoryRoot, ReadOnly>,
    target_path: &str,
) -> Result<(Vec<u8>, u32), String> {
    use std::io::Read;
    let relative = crate::platform::RelativePath::parse(target_path)
        .map_err(|error| format!("managed path {target_path:?} is invalid: {error}"))?;
    let target = authority
        .resolve_read(&relative, crate::platform::ObjectClass::RegularFile)
        .map_err(|error| format!("cannot resolve the managed file {target_path}: {error}"))?;
    let mut handle = target
        .try_clone_file()
        .map_err(|error| format!("cannot open the managed file {target_path}: {error}"))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        handle
            .metadata()
            .map_err(|error| format!("cannot inspect the managed file {target_path}: {error}"))?
            .permissions()
            .mode()
            & 0o7777
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read the managed file {target_path}: {error}"))?;
    Ok((bytes, mode))
}

/// One composed destination-file group.
struct ComposedGroup {
    path: crate::repository::RelativePath,
    target: String,
    /// The frozen identity of an existing target; `None` is the lawful
    /// creation case.
    observed: Option<crate::repository::FileIdentity>,
    current_bytes: Vec<u8>,
    replacement: Vec<u8>,
}

/// Compose one destination-file group: resolve the frozen identity, read
/// the destination once (an absent target composes from empty bytes and
/// is created by the pass), and build the complete replacement bytes.
/// Any failure is returned as the group's reason and contained to the
/// group.
fn compose_group(
    working: &Path,
    snapshot: &RepositorySnapshot,
    source_roots: &std::collections::HashMap<
        String,
        AuthorityRoot<crate::platform::SourceSnapshotRoot, ReadOnly>,
    >,
    target: &str,
    members: &[&crate::lifecycle::sync_plan::PlanItem],
) -> Result<ComposedGroup, String> {
    let Some(entry) = snapshot
        .targets()
        .iter()
        .find(|target_entry| target_entry.path().as_bytes() == target.as_bytes())
    else {
        return Err(format!(
            "managed target {target} is not frozen in the snapshot"
        ));
    };
    let observed = entry.observed_file().cloned();
    let current_bytes = match &observed {
        Some(_) => std::fs::read(working.join(target))
            .map_err(|error| format!("cannot read the current managed target {target}: {error}"))?,
        None => Vec::new(),
    };
    let whole_file = members
        .iter()
        .any(|member| member.kind == crate::source::ItemKind::WholeFile);
    let replacement = if whole_file {
        // Resolution keeps targets homogeneous; a mixed or multi-claim
        // whole-file group cannot reach a valid plan.
        if members.len() != 1 {
            return Err(format!(
                "target {target} carries a whole-file claim beside other items"
            ));
        }
        authoritative_bytes(source_roots, members[0])?
    } else {
        // Named sections: the whole source file is the section body;
        // the delimiter syntax follows the destination file format.
        let syntax = crate::managed_content::lookup_by_extension(target)
            .map_err(|error| format!("no delimiter syntax for destination {target}: {error}"))?;
        let mut writes = Vec::with_capacity(members.len());
        for member in members {
            let Some(section) = member.section.clone() else {
                return Err(format!("section item {} has no section id", member.id));
            };
            writes.push(crate::managed_content::SectionWrite {
                id: section,
                payload: authoritative_bytes(source_roots, member)?,
            });
        }
        crate::managed_content::apply_sections(&current_bytes, syntax, &writes)
            .map_err(|error| format!("target {target}: {error}"))?
            .content
    };
    Ok(ComposedGroup {
        path: entry.path().clone(),
        target: target.to_owned(),
        observed,
        current_bytes,
        replacement,
    })
}

/// Read one plan item's authoritative bytes: the exact source file.
/// Whole files replace the destination; section files are the exact
/// section body.
fn authoritative_bytes(
    source_roots: &std::collections::HashMap<
        String,
        AuthorityRoot<crate::platform::SourceSnapshotRoot, ReadOnly>,
    >,
    item: &crate::lifecycle::sync_plan::PlanItem,
) -> Result<Vec<u8>, String> {
    let root = source_roots
        .get(&item.source)
        .ok_or_else(|| format!("source {} has no opened authority root", item.source))?;
    read_source_file(root, &item.source_path)
}
