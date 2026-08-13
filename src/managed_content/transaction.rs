//! A pure state machine for one managed-file replacement.
//!
//! This module records the protocol.  It does not open files, create
//! directories, write bytes, rename paths, or promise a stronger durability
//! boundary than the managed-content and fleet-lifecycle contracts provide.
//! The filesystem implementation can use these states as its journal-facing
//! vocabulary and can reject an operation before it performs an invalid step.
//! Path checks here are lexical and portable only.  Canonical no-follow
//! filesystem identity, containment, mount, alias, and object checks remain
//! the authority-adapter boundary owned by the .8 workstream.

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

/// The comparison made against the frozen authoritative bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Comparison {
    /// Destination bytes already equal the authoritative bytes.
    Equal,
    /// Destination bytes differ and require an atomic replacement.
    Different,
}

/// The complete-content visibility named by the replacement contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentVisibility {
    Old,
    Equal,
    New,
}

impl ContentVisibility {
    pub fn label(self) -> &'static str {
        match self {
            Self::Old => "old-complete",
            Self::Equal => "equal-noop",
            Self::New => "new-complete",
        }
    }
}

/// The externally visible protocol states for one target file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionState {
    /// The target was observed before comparison or replacement.
    Old,
    /// The authoritative and destination bytes compare equal.
    Equal,
    /// The target differs and a replacement is required.
    New,
    /// A candidate name has been reserved, but its creation is not confirmed.
    TempCreatedPending,
    /// A candidate name was occupied by an artifact not owned by this run.
    TempCollision,
    /// The operation owns a same-directory temporary artifact.
    TempCreated,
    /// Complete authoritative bytes were written to the temporary artifact.
    ContentWritten,
    /// The temporary file's complete bytes were synchronized.
    ContentSynchronized,
    /// Safe metadata was applied to the temporary artifact.
    MetadataApplied,
    /// The temporary artifact was atomically renamed over the target.
    Renamed,
    /// The containing directory was synchronized after the rename.
    ParentSynchronized,
    /// Cleanup is pending or has completed; the checkpoint distinguishes them.
    Cleanup,
    /// Cleanup completed after a failed replacement and the failure is terminal.
    Failed,
    /// The operation stopped before a terminal outcome.
    Interrupted,
    /// Durable evidence was observed after an interruption.
    Recovered,
    /// The operation reached a terminal synchronized outcome.
    Synced,
}

/// Named checkpoints written by a transaction journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalCheckpoint {
    Prepared,
    Compared,
    TempCreationStarted,
    TempCollision,
    TempCreated,
    ContentWritten,
    ContentSynchronized,
    MetadataApplied,
    Renamed,
    ParentSynchronized,
    CleanupStarted,
    CleanupCompleted,
    Interrupted,
    Recovered,
    CleanupRequired,
    CleanupFailed,
    Failed,
    Synced,
}

impl JournalCheckpoint {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Compared => "compared",
            Self::TempCreationStarted => "temp-creation-started",
            Self::TempCollision => "temp-collision",
            Self::TempCreated => "temp-created",
            Self::ContentWritten => "content-written",
            Self::ContentSynchronized => "content-synchronized",
            Self::MetadataApplied => "metadata-applied",
            Self::Renamed => "renamed",
            Self::ParentSynchronized => "parent-synchronized",
            Self::CleanupStarted => "cleanup-started",
            Self::CleanupCompleted => "cleanup-completed",
            Self::Interrupted => "interrupted",
            Self::Recovered => "recovered",
            Self::CleanupRequired => "cleanup-required",
            Self::CleanupFailed => "cleanup-failed",
            Self::Failed => "failed",
            Self::Synced => "synced",
        }
    }
}

/// Parent directories that the current operation may remove on failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParentDirectories {
    Existing,
    Created(Vec<PathBuf>),
}

impl ParentDirectories {
    pub fn existing() -> Self {
        Self::Existing
    }

    pub fn created<I, S>(parents: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<PathBuf>,
    {
        Self::Created(parents.into_iter().map(Into::into).collect())
    }

    fn requires_cleanup(&self) -> bool {
        matches!(self, Self::Created(parents) if !parents.is_empty())
    }
}

/// The plan identity used to make temporary names operation-local.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionPlan {
    operation_id: String,
    target: PathBuf,
    parents: ParentDirectories,
}

impl TransactionPlan {
    pub fn new(
        operation_id: impl Into<String>,
        target: PathBuf,
        parents: ParentDirectories,
    ) -> Result<Self, PlanError> {
        let operation_id = operation_id.into();
        if operation_id.is_empty() {
            return Err(PlanError::EmptyOperationId);
        }
        let target_components = validate_relative_path(&target, "target")?;
        let target_parent_len = target_components.len().saturating_sub(1);
        if let ParentDirectories::Created(created) = &parents {
            let mut validated = Vec::with_capacity(created.len());
            for parent in created {
                let components = validate_relative_path(parent, "created parent")?;
                if components.len() > target_parent_len
                    || components != target_components[..components.len()]
                {
                    return Err(PlanError::ParentOutsideTarget {
                        path: parent.display().to_string(),
                    });
                }
                if validated
                    .iter()
                    .any(|existing: &Vec<String>| existing == &components)
                {
                    return Err(PlanError::DuplicateParent {
                        path: parent.display().to_string(),
                    });
                }
                validated.push(components);
            }
        }
        Ok(Self {
            operation_id,
            target,
            parents,
        })
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn parents(&self) -> &ParentDirectories {
        &self.parents
    }

    fn target_parent(&self) -> Option<&Path> {
        self.target.parent()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    EmptyOperationId,
    EmptyPath { field: &'static str },
    InvalidUtf8 { field: &'static str },
    AbsolutePath { field: &'static str, path: String },
    InvalidSeparator { field: &'static str, path: String },
    EmptyComponent { field: &'static str, path: String },
    CurrentDirectoryComponent { field: &'static str, path: String },
    ParentTraversal { field: &'static str, path: String },
    WindowsPrefix { field: &'static str, path: String },
    ParentOutsideTarget { path: String },
    DuplicateParent { path: String },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyOperationId => formatter.write_str("transaction operation id is empty"),
            Self::EmptyPath { field } => write!(formatter, "{field} path is empty"),
            Self::InvalidUtf8 { field } => write!(formatter, "{field} path is not UTF-8"),
            Self::AbsolutePath { field, path } => {
                write!(formatter, "{field} path is absolute: {path:?}")
            }
            Self::InvalidSeparator { field, path } => {
                write!(
                    formatter,
                    "{field} path uses an unsupported separator: {path:?}"
                )
            }
            Self::EmptyComponent { field, path } => {
                write!(formatter, "{field} path has an empty component: {path:?}")
            }
            Self::CurrentDirectoryComponent { field, path } => {
                write!(
                    formatter,
                    "{field} path has a current-directory component: {path:?}"
                )
            }
            Self::ParentTraversal { field, path } => {
                write!(formatter, "{field} path traverses a parent: {path:?}")
            }
            Self::WindowsPrefix { field, path } => {
                write!(formatter, "{field} path has a drive prefix: {path:?}")
            }
            Self::ParentOutsideTarget { path } => {
                write!(
                    formatter,
                    "created parent is outside the target path: {path:?}"
                )
            }
            Self::DuplicateParent { path } => {
                write!(formatter, "created parent is duplicated: {path:?}")
            }
        }
    }
}

impl Error for PlanError {}

fn validate_relative_path(path: &Path, field: &'static str) -> Result<Vec<String>, PlanError> {
    let raw = path.to_str().ok_or(PlanError::InvalidUtf8 { field })?;
    if raw.is_empty() {
        return Err(PlanError::EmptyPath { field });
    }
    if raw.starts_with('/') {
        return Err(PlanError::AbsolutePath {
            field,
            path: raw.to_owned(),
        });
    }
    if raw.contains('\\') {
        return Err(PlanError::InvalidSeparator {
            field,
            path: raw.to_owned(),
        });
    }
    let mut components = Vec::new();
    for (index, component) in raw.split('/').enumerate() {
        if component.is_empty() {
            return Err(PlanError::EmptyComponent {
                field,
                path: raw.to_owned(),
            });
        }
        if component == "." {
            return Err(PlanError::CurrentDirectoryComponent {
                field,
                path: raw.to_owned(),
            });
        }
        if component == ".." {
            return Err(PlanError::ParentTraversal {
                field,
                path: raw.to_owned(),
            });
        }
        if index == 0 && component.ends_with(':') {
            return Err(PlanError::WindowsPrefix {
                field,
                path: raw.to_owned(),
            });
        }
        components.push(component.to_owned());
    }
    Ok(components)
}

/// A candidate temporary path and its strictly increasing collision attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempCandidate {
    path: PathBuf,
    attempt: u32,
}

impl TempCandidate {
    pub fn new(path: PathBuf, attempt: u32) -> Result<Self, CandidateError> {
        if path.as_os_str().is_empty() {
            return Err(CandidateError::EmptyPath);
        }
        validate_relative_path(&path, "temporary candidate")
            .map_err(CandidateError::InvalidPath)?;
        if attempt == 0 {
            return Err(CandidateError::ZeroAttempt);
        }
        Ok(Self { path, attempt })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateError {
    EmptyPath,
    ZeroAttempt,
    EmptyOwnerToken,
    InvalidPath(PlanError),
}

impl fmt::Display for CandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("temporary candidate path is empty"),
            Self::ZeroAttempt => {
                formatter.write_str("temporary candidate attempt must be positive")
            }
            Self::EmptyOwnerToken => formatter.write_str("temporary owner token is empty"),
            Self::InvalidPath(error) => error.fmt(formatter),
        }
    }
}

impl Error for CandidateError {}

/// A temporary artifact after exclusive creation succeeded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempArtifact {
    candidate: TempCandidate,
    owner_token: String,
}

impl TempArtifact {
    pub fn new(
        candidate: TempCandidate,
        owner_token: impl Into<String>,
    ) -> Result<Self, CandidateError> {
        let owner_token = owner_token.into();
        if owner_token.is_empty() {
            return Err(CandidateError::EmptyOwnerToken);
        }
        Ok(Self {
            candidate,
            owner_token,
        })
    }

    pub fn candidate(&self) -> &TempCandidate {
        &self.candidate
    }

    pub fn owner_token(&self) -> &str {
        &self.owner_token
    }
}

/// The result of the metadata stage, before the replacement rename.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataResult {
    Preserved,
    NotRequired,
}

/// Evidence observed while replaying an interrupted operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryObservation {
    OldComplete,
    NewComplete,
    /// A same-operation temporary artifact observed after exclusive creation
    /// but before the `temp-created` journal checkpoint was durable.
    TempOnly(TempArtifact),
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryResult {
    OldComplete,
    NewComplete,
    TempOnly,
    Unknown,
}

impl From<RecoveryObservation> for RecoveryResult {
    fn from(value: RecoveryObservation) -> Self {
        match value {
            RecoveryObservation::OldComplete => Self::OldComplete,
            RecoveryObservation::NewComplete => Self::NewComplete,
            RecoveryObservation::TempOnly(_) => Self::TempOnly,
            RecoveryObservation::Unknown => Self::Unknown,
        }
    }
}

/// Which residue may be removed by the current operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupDisposition {
    None,
    ParentsOnly,
    TempOnly,
    TempAndParents,
}

/// Evidence that cleanup completed without widening its authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupResult {
    NothingToRemove,
    TempConsumed,
    TempRemoved,
    ParentsRemoved,
    TempAndParentsRemoved,
}

impl CleanupResult {
    fn matches(self, disposition: CleanupDisposition) -> bool {
        match disposition {
            CleanupDisposition::None => {
                matches!(self, Self::NothingToRemove | Self::TempConsumed)
            }
            CleanupDisposition::ParentsOnly => matches!(self, Self::ParentsRemoved),
            CleanupDisposition::TempOnly => matches!(self, Self::TempRemoved),
            CleanupDisposition::TempAndParents => {
                matches!(self, Self::TempAndParentsRemoved)
            }
        }
    }
}

/// The operation stage that failed before replacement completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    TempCreation,
    ContentWrite,
    ContentSynchronization,
    Metadata,
    Rename,
    ParentSynchronization,
    Cleanup,
}

/// Typed evidence for a failed stage and the residue it still owns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureEvidence {
    kind: FailureKind,
    reason: String,
    last_durable_checkpoint: JournalCheckpoint,
    residue: CleanupDisposition,
}

impl FailureEvidence {
    pub fn kind(&self) -> FailureKind {
        self.kind
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn last_durable_checkpoint(&self) -> JournalCheckpoint {
        self.last_durable_checkpoint
    }

    pub fn residue(&self) -> CleanupDisposition {
        self.residue
    }
}

/// The next action selected by a recovery observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryNextAction {
    RetryReplacement,
    ProveNewDurability,
    CleanupOwnedResidue,
    Investigate,
}

/// The identity captured at the interruption boundary.
///
/// A proof is valid only for the exact immutable plan and operation journal
/// that produced this binding. Keeping the snapshot on the transaction lets
/// cleanup append later checkpoints without changing what the recovery
/// evidence describes.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RecoveryBinding {
    plan: TransactionPlan,
    operation_id: String,
    candidate: Option<TempCandidate>,
    interrupted_at: JournalCheckpoint,
    journal: Vec<JournalCheckpoint>,
}

/// Evidence required before a recovered new-complete target can be successful.
///
/// There is intentionally no public constructor from an arbitrary checkpoint
/// slice.  The transaction creates this value only from its own recovery
/// binding, so a complete-looking slice from another operation or plan cannot
/// be accepted as evidence for this one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryDurabilityProof {
    binding: RecoveryBinding,
    content_synchronized: bool,
    metadata_applied: bool,
    renamed: bool,
    parent_synchronized: bool,
}

impl RecoveryDurabilityProof {
    fn from_binding(binding: RecoveryBinding) -> Result<Self, ProofError> {
        let checkpoints = &binding.journal;
        let content_synchronized =
            ordered_checkpoint(checkpoints, JournalCheckpoint::ContentSynchronized, 0).is_some();
        let metadata_applied =
            ordered_checkpoint(checkpoints, JournalCheckpoint::MetadataApplied, 1).is_some();
        let renamed = ordered_checkpoint(checkpoints, JournalCheckpoint::Renamed, 2).is_some();
        let parent_synchronized =
            ordered_checkpoint(checkpoints, JournalCheckpoint::ParentSynchronized, 3).is_some();
        let proof = Self {
            binding,
            content_synchronized,
            metadata_applied,
            renamed,
            parent_synchronized,
        };
        if proof.is_complete() {
            Ok(proof)
        } else {
            Err(ProofError::Incomplete)
        }
    }

    fn is_complete(&self) -> bool {
        self.content_synchronized
            && self.metadata_applied
            && self.renamed
            && self.parent_synchronized
    }
}

fn ordered_checkpoint(
    checkpoints: &[JournalCheckpoint],
    expected: JournalCheckpoint,
    stage: usize,
) -> Option<usize> {
    let required = [
        JournalCheckpoint::ContentSynchronized,
        JournalCheckpoint::MetadataApplied,
        JournalCheckpoint::Renamed,
        JournalCheckpoint::ParentSynchronized,
    ];
    let mut start = 0;
    for checkpoint in required.into_iter().take(stage) {
        let position = checkpoints[start..]
            .iter()
            .position(|candidate| *candidate == checkpoint)?;
        start += position + 1;
    }
    checkpoints[start..]
        .iter()
        .position(|candidate| *candidate == expected)
        .map(|position| start + position)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofError {
    Incomplete,
    NotNewCompleteRecovery,
    MissingRecoveryBinding,
}

impl fmt::Display for ProofError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete => formatter.write_str("recovery durability proof is incomplete"),
            Self::NotNewCompleteRecovery => {
                formatter.write_str("transaction is not a recovered new-complete cleanup")
            }
            Self::MissingRecoveryBinding => {
                formatter.write_str("recovery durability binding is missing")
            }
        }
    }
}

impl Error for ProofError {}

/// A precise invalid transition or artifact mismatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    InvalidTransition {
        from: TransactionState,
        operation: &'static str,
    },
    CandidateMismatch,
    ForeignTempOwner,
    TempOutsideTargetDirectory,
    TempAttemptNotIncreasing,
    CleanupResultMismatch {
        outstanding: CleanupDisposition,
        reported: CleanupResult,
    },
    FailureNotRecorded,
    DurabilityProofRequired,
    DurabilityProofMismatch,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { from, operation } => {
                write!(formatter, "cannot {operation} from {from:?} state")
            }
            Self::CandidateMismatch => {
                formatter.write_str("temporary artifact does not match the reserved candidate")
            }
            Self::ForeignTempOwner => {
                formatter.write_str("temporary artifact belongs to another operation")
            }
            Self::TempOutsideTargetDirectory => {
                formatter.write_str("temporary candidate is outside the target directory")
            }
            Self::TempAttemptNotIncreasing => {
                formatter.write_str("temporary candidate attempt is not strictly increasing")
            }
            Self::CleanupResultMismatch {
                outstanding,
                reported,
            } => write!(
                formatter,
                "cleanup result {reported:?} does not match outstanding {outstanding:?}"
            ),
            Self::FailureNotRecorded => {
                formatter.write_str("cannot finalize failure without recorded failure evidence")
            }
            Self::DurabilityProofRequired => {
                formatter.write_str("recovered new-complete content requires durability proof")
            }
            Self::DurabilityProofMismatch => {
                formatter.write_str("recovery durability proof belongs to another transaction")
            }
        }
    }
}

impl Error for TransactionError {}

/// A pure, replayable transaction protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction {
    plan: TransactionPlan,
    state: TransactionState,
    checkpoint: JournalCheckpoint,
    journal: Vec<JournalCheckpoint>,
    candidate: Option<TempCandidate>,
    artifact: Option<TempArtifact>,
    last_attempt: u32,
    collision_artifacts: Vec<TempCandidate>,
    cleanup: CleanupDisposition,
    cleanup_result: Option<CleanupResult>,
    interrupted_at: Option<JournalCheckpoint>,
    interruption_reason: Option<String>,
    recovery_binding: Option<RecoveryBinding>,
    recovery: Option<RecoveryResult>,
    recovery_action: Option<RecoveryNextAction>,
    renamed: bool,
    failure: Option<FailureEvidence>,
    failures: Vec<FailureEvidence>,
}

impl Transaction {
    pub fn begin(plan: TransactionPlan) -> Self {
        let cleanup = if plan.parents.requires_cleanup() {
            CleanupDisposition::ParentsOnly
        } else {
            CleanupDisposition::None
        };
        Self {
            plan,
            state: TransactionState::Old,
            checkpoint: JournalCheckpoint::Prepared,
            journal: vec![JournalCheckpoint::Prepared],
            candidate: None,
            artifact: None,
            last_attempt: 0,
            collision_artifacts: Vec::new(),
            cleanup,
            cleanup_result: None,
            interrupted_at: None,
            interruption_reason: None,
            recovery_binding: None,
            recovery: None,
            recovery_action: None,
            renamed: false,
            failure: None,
            failures: Vec::new(),
        }
    }

    pub fn state(&self) -> &TransactionState {
        &self.state
    }

    pub fn checkpoint(&self) -> JournalCheckpoint {
        self.checkpoint
    }

    pub fn cleanup_disposition(&self) -> CleanupDisposition {
        self.cleanup
    }

    pub fn cleanup_result(&self) -> Option<CleanupResult> {
        self.cleanup_result
    }

    pub fn failure_reason(&self) -> Option<&str> {
        self.failure
            .as_ref()
            .map(FailureEvidence::reason)
            .or(self.interruption_reason.as_deref())
    }

    pub fn failure(&self) -> Option<&FailureEvidence> {
        self.failure.as_ref()
    }

    pub fn failures(&self) -> &[FailureEvidence] {
        &self.failures
    }

    pub fn interrupted_at(&self) -> Option<JournalCheckpoint> {
        self.interrupted_at
    }

    pub fn recovery_result(&self) -> Option<RecoveryResult> {
        self.recovery
    }

    pub fn recovery_action(&self) -> Option<RecoveryNextAction> {
        self.recovery_action
    }

    /// Build recovery evidence from this transaction's exact interruption
    /// journal.  Arbitrary checkpoint slices are not accepted as proofs.
    pub fn durability_proof(&self) -> Result<RecoveryDurabilityProof, ProofError> {
        if self.state != TransactionState::Cleanup
            || self.recovery != Some(RecoveryResult::NewComplete)
            || self.recovery_action != Some(RecoveryNextAction::ProveNewDurability)
            || self.checkpoint != JournalCheckpoint::CleanupCompleted
            || self.cleanup != CleanupDisposition::None
        {
            return Err(ProofError::NotNewCompleteRecovery);
        }
        let binding = self
            .recovery_binding
            .clone()
            .ok_or(ProofError::MissingRecoveryBinding)?;
        RecoveryDurabilityProof::from_binding(binding)
    }

    pub fn last_durable_checkpoint(&self) -> Option<JournalCheckpoint> {
        self.failure
            .as_ref()
            .map(FailureEvidence::last_durable_checkpoint)
            .or(self.interrupted_at)
    }

    pub fn collisions(&self) -> usize {
        self.collision_artifacts.len()
    }

    pub fn collision_candidates(&self) -> &[TempCandidate] {
        &self.collision_artifacts
    }

    pub fn was_renamed(&self) -> bool {
        self.renamed
    }

    pub fn journal_labels(&self) -> Vec<&'static str> {
        self.journal
            .iter()
            .map(|checkpoint| checkpoint.label())
            .collect()
    }

    pub fn compare(mut self, comparison: Comparison) -> Result<Self, TransactionError> {
        self.require(TransactionState::Old, "compare")?;
        self.record_checkpoint(JournalCheckpoint::Compared);
        self.state = match comparison {
            Comparison::Equal => TransactionState::Equal,
            Comparison::Different => TransactionState::New,
        };
        if self.state == TransactionState::Equal {
            self.cleanup = CleanupDisposition::None;
        }
        Ok(self)
    }

    pub fn finish_equal(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::Equal, "finish_equal")?;
        self.cleanup_result = Some(CleanupResult::NothingToRemove);
        self.cleanup = CleanupDisposition::None;
        self.state = TransactionState::Synced;
        self.record_checkpoint(JournalCheckpoint::Synced);
        Ok(self)
    }

    pub fn reserve_temp(mut self, candidate: TempCandidate) -> Result<Self, TransactionError> {
        if self.state == TransactionState::TempCreatedPending
            && candidate.attempt() <= self.last_attempt
        {
            return Err(TransactionError::TempAttemptNotIncreasing);
        }
        if !matches!(
            self.state,
            TransactionState::New | TransactionState::TempCollision
        ) {
            return Err(self.invalid("reserve_temp"));
        }
        if candidate.path().parent() != self.plan.target_parent()
            || candidate.path() == self.plan.target()
        {
            return Err(TransactionError::TempOutsideTargetDirectory);
        }
        if candidate.attempt() <= self.last_attempt {
            return Err(TransactionError::TempAttemptNotIncreasing);
        }
        self.last_attempt = candidate.attempt();
        self.candidate = Some(candidate);
        self.artifact = None;
        self.state = TransactionState::TempCreatedPending;
        self.record_checkpoint(JournalCheckpoint::TempCreationStarted);
        Ok(self)
    }

    /// Record an I/O failure while creating the reserved temporary artifact.
    ///
    /// The candidate was never proven to be owned, so only the cleanup
    /// obligation that existed before creation remains authorized.
    pub fn temp_creation_failed(self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.fail_stage(
            TransactionState::TempCreatedPending,
            FailureKind::TempCreation,
            "temp_creation_failed",
            reason,
        )
    }

    pub fn temp_collision(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::TempCreatedPending, "temp_collision")?;
        let candidate = self
            .candidate
            .take()
            .ok_or(TransactionError::CandidateMismatch)?;
        self.collision_artifacts.push(candidate);
        self.state = TransactionState::TempCollision;
        self.record_checkpoint(JournalCheckpoint::TempCollision);
        Ok(self)
    }

    pub fn temp_created(mut self, artifact: TempArtifact) -> Result<Self, TransactionError> {
        self.require(TransactionState::TempCreatedPending, "temp_created")?;
        if self.candidate.as_ref() != Some(artifact.candidate()) {
            return Err(TransactionError::CandidateMismatch);
        }
        if artifact.owner_token() != self.plan.operation_id() {
            return Err(TransactionError::ForeignTempOwner);
        }
        self.artifact = Some(artifact);
        self.state = TransactionState::TempCreated;
        self.cleanup = if self.plan.parents.requires_cleanup() {
            CleanupDisposition::TempAndParents
        } else {
            CleanupDisposition::TempOnly
        };
        self.record_checkpoint(JournalCheckpoint::TempCreated);
        Ok(self)
    }

    pub fn content_written(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::TempCreated, "content_written")?;
        self.state = TransactionState::ContentWritten;
        self.record_checkpoint(JournalCheckpoint::ContentWritten);
        Ok(self)
    }

    pub fn content_write_failed(self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.fail_stage(
            TransactionState::TempCreated,
            FailureKind::ContentWrite,
            "content_write_failed",
            reason,
        )
    }

    pub fn content_synchronized(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::ContentWritten, "content_synchronized")?;
        self.state = TransactionState::ContentSynchronized;
        self.record_checkpoint(JournalCheckpoint::ContentSynchronized);
        Ok(self)
    }

    pub fn content_sync_failed(self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.fail_stage(
            TransactionState::ContentWritten,
            FailureKind::ContentSynchronization,
            "content_sync_failed",
            reason,
        )
    }

    pub fn metadata_applied(mut self, _result: MetadataResult) -> Result<Self, TransactionError> {
        self.require(TransactionState::ContentSynchronized, "metadata_applied")?;
        self.state = TransactionState::MetadataApplied;
        self.record_checkpoint(JournalCheckpoint::MetadataApplied);
        Ok(self)
    }

    pub fn metadata_failed(self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.fail_stage(
            TransactionState::ContentSynchronized,
            FailureKind::Metadata,
            "metadata_failed",
            reason,
        )
    }

    pub fn rename_failed(self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.fail_stage(
            TransactionState::MetadataApplied,
            FailureKind::Rename,
            "rename_failed",
            reason,
        )
    }

    pub fn renamed(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::MetadataApplied, "renamed")?;
        self.state = TransactionState::Renamed;
        self.renamed = true;
        self.cleanup = CleanupDisposition::None;
        self.record_checkpoint(JournalCheckpoint::Renamed);
        Ok(self)
    }

    pub fn parent_sync_failed(self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.fail_stage(
            TransactionState::Renamed,
            FailureKind::ParentSynchronization,
            "parent_sync_failed",
            reason,
        )
    }

    pub fn parent_synchronized(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::Renamed, "parent_synchronized")?;
        self.state = TransactionState::ParentSynchronized;
        self.record_checkpoint(JournalCheckpoint::ParentSynchronized);
        Ok(self)
    }

    pub fn cleanup(mut self, result: CleanupResult) -> Result<Self, TransactionError> {
        if !matches!(
            self.state,
            TransactionState::ParentSynchronized
                | TransactionState::Recovered
                | TransactionState::Cleanup
        ) {
            return Err(self.invalid("cleanup"));
        }
        if self.checkpoint == JournalCheckpoint::CleanupCompleted {
            return Err(self.invalid("cleanup"));
        }
        if !result.matches(self.cleanup) {
            return Err(TransactionError::CleanupResultMismatch {
                outstanding: self.cleanup,
                reported: result,
            });
        }
        if self.checkpoint != JournalCheckpoint::CleanupStarted {
            self.record_checkpoint(JournalCheckpoint::CleanupStarted);
        }
        self.cleanup_result = Some(result);
        self.cleanup = CleanupDisposition::None;
        self.state = TransactionState::Cleanup;
        self.record_checkpoint(JournalCheckpoint::CleanupCompleted);
        Ok(self)
    }

    pub fn cleanup_started(mut self) -> Result<Self, TransactionError> {
        if !matches!(
            self.state,
            TransactionState::ParentSynchronized
                | TransactionState::Recovered
                | TransactionState::Cleanup
        ) {
            return Err(self.invalid("cleanup_started"));
        }
        if self.checkpoint == JournalCheckpoint::CleanupCompleted {
            return Err(self.invalid("cleanup_started"));
        }
        if self.checkpoint != JournalCheckpoint::CleanupStarted {
            self.record_checkpoint(JournalCheckpoint::CleanupStarted);
        }
        self.state = TransactionState::Cleanup;
        Ok(self)
    }

    pub fn cleanup_failed(mut self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        self.require(TransactionState::Cleanup, "cleanup_failed")?;
        if self.checkpoint == JournalCheckpoint::CleanupCompleted {
            return Err(self.invalid("cleanup_failed"));
        }
        let evidence = FailureEvidence {
            kind: FailureKind::Cleanup,
            reason: reason.into(),
            last_durable_checkpoint: self.checkpoint,
            residue: self.cleanup,
        };
        self.failure = Some(evidence.clone());
        self.failures.push(evidence);
        self.state = TransactionState::Failed;
        self.record_checkpoint(JournalCheckpoint::CleanupFailed);
        Ok(self)
    }

    pub fn finish(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::Cleanup, "finish")?;
        if self.checkpoint != JournalCheckpoint::CleanupCompleted {
            return Err(TransactionError::InvalidTransition {
                from: self.state,
                operation: "finish",
            });
        }
        if self.failure.is_some() || self.recovery.is_some() {
            return Err(self.invalid("finish"));
        }
        self.state = TransactionState::Synced;
        self.record_checkpoint(JournalCheckpoint::Synced);
        Ok(self)
    }

    pub fn finish_failure(mut self) -> Result<Self, TransactionError> {
        self.require(TransactionState::Cleanup, "finish_failure")?;
        if self.checkpoint != JournalCheckpoint::CleanupCompleted {
            return Err(TransactionError::InvalidTransition {
                from: self.state,
                operation: "finish_failure",
            });
        }
        if self.failure.is_none() {
            return Err(TransactionError::FailureNotRecorded);
        }
        self.state = TransactionState::Failed;
        self.record_checkpoint(JournalCheckpoint::Failed);
        Ok(self)
    }

    pub fn interrupt(mut self, reason: impl Into<String>) -> Result<Self, TransactionError> {
        if matches!(
            self.state,
            TransactionState::Synced
                | TransactionState::Failed
                | TransactionState::Interrupted
                | TransactionState::Recovered
        ) || (self.state == TransactionState::Cleanup
            && self.checkpoint == JournalCheckpoint::CleanupCompleted)
        {
            return Err(self.invalid("interrupt"));
        }
        self.interruption_reason = Some(reason.into());
        self.interrupted_at = Some(self.checkpoint);
        self.state = TransactionState::Interrupted;
        self.record_checkpoint(JournalCheckpoint::Interrupted);
        Ok(self)
    }

    pub fn recover(mut self, observation: RecoveryObservation) -> Result<Self, TransactionError> {
        self.require(TransactionState::Interrupted, "recover")?;
        let interrupted_at = self
            .interrupted_at
            .expect("interrupted transactions retain their durable boundary");
        self.recovery_binding = Some(RecoveryBinding {
            plan: self.plan.clone(),
            operation_id: self.plan.operation_id.clone(),
            candidate: self.candidate.clone(),
            interrupted_at,
            journal: self.journal.clone(),
        });
        self.recovery = Some(observation.clone().into());
        self.recovery_action = Some(match observation {
            RecoveryObservation::OldComplete => RecoveryNextAction::RetryReplacement,
            RecoveryObservation::NewComplete => {
                self.artifact = None;
                self.candidate = None;
                // A complete target is the authoritative new file.  Its
                // parent directories are part of the successful target's
                // containment, never residue that recovery may remove.
                self.cleanup = CleanupDisposition::None;
                RecoveryNextAction::ProveNewDurability
            }
            RecoveryObservation::TempOnly(artifact) => {
                self.validate_recovery_artifact(&artifact)?;
                self.artifact = Some(artifact);
                self.cleanup = self.owned_cleanup_disposition();
                RecoveryNextAction::CleanupOwnedResidue
            }
            RecoveryObservation::Unknown => RecoveryNextAction::Investigate,
        });
        self.state = TransactionState::Recovered;
        self.record_checkpoint(JournalCheckpoint::Recovered);
        Ok(self)
    }

    pub fn finish_recovered_new(
        mut self,
        proof: RecoveryDurabilityProof,
    ) -> Result<Self, TransactionError> {
        self.require(TransactionState::Cleanup, "finish_recovered_new")?;
        if self.recovery != Some(RecoveryResult::NewComplete)
            || self.recovery_action != Some(RecoveryNextAction::ProveNewDurability)
            || self.checkpoint != JournalCheckpoint::CleanupCompleted
            || self.cleanup != CleanupDisposition::None
            || self.recovery_binding.is_none()
            || !proof.is_complete()
        {
            return Err(TransactionError::DurabilityProofRequired);
        }
        if self.recovery_binding.as_ref() != Some(&proof.binding) {
            return Err(TransactionError::DurabilityProofMismatch);
        }
        self.state = TransactionState::Synced;
        self.record_checkpoint(JournalCheckpoint::Synced);
        Ok(self)
    }

    fn require(
        &self,
        expected: TransactionState,
        operation: &'static str,
    ) -> Result<(), TransactionError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(self.invalid(operation))
        }
    }

    fn invalid(&self, operation: &'static str) -> TransactionError {
        TransactionError::InvalidTransition {
            from: self.state,
            operation,
        }
    }

    fn owned_cleanup_disposition(&self) -> CleanupDisposition {
        match (
            self.artifact.is_some(),
            self.plan.parents.requires_cleanup(),
        ) {
            (false, false) => CleanupDisposition::None,
            (false, true) => CleanupDisposition::ParentsOnly,
            (true, false) => CleanupDisposition::TempOnly,
            (true, true) => CleanupDisposition::TempAndParents,
        }
    }

    fn validate_recovery_artifact(&self, artifact: &TempArtifact) -> Result<(), TransactionError> {
        if self.candidate.as_ref() != Some(artifact.candidate()) {
            return Err(TransactionError::CandidateMismatch);
        }
        if artifact.owner_token() != self.plan.operation_id() {
            return Err(TransactionError::ForeignTempOwner);
        }
        Ok(())
    }

    fn fail_stage(
        mut self,
        expected: TransactionState,
        kind: FailureKind,
        operation: &'static str,
        reason: impl Into<String>,
    ) -> Result<Self, TransactionError> {
        self.require(expected, operation)?;
        let evidence = FailureEvidence {
            kind,
            reason: reason.into(),
            last_durable_checkpoint: self.checkpoint,
            residue: self.cleanup,
        };
        self.failure = Some(evidence.clone());
        self.failures.push(evidence);
        self.state = TransactionState::Cleanup;
        self.record_checkpoint(JournalCheckpoint::CleanupRequired);
        Ok(self)
    }

    fn record_checkpoint(&mut self, checkpoint: JournalCheckpoint) {
        self.checkpoint = checkpoint;
        self.journal.push(checkpoint);
    }
}
