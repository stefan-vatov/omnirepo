use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The result of the metadata stage, before the replacement rename.
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
    pub(crate) fn matches(self, disposition: CleanupDisposition) -> bool {
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
    pub(crate) kind: FailureKind,
    pub(crate) reason: String,
    pub(crate) last_durable_checkpoint: JournalCheckpoint,
    pub(crate) residue: CleanupDisposition,
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
pub(crate) struct RecoveryBinding {
    pub(crate) plan: TransactionPlan,
    pub(crate) operation_id: String,
    pub(crate) candidate: Option<TempCandidate>,
    pub(crate) interrupted_at: JournalCheckpoint,
    pub(crate) journal: Vec<JournalCheckpoint>,
}

/// Evidence required before a recovered new-complete target can be successful.
///
/// There is intentionally no public constructor from an arbitrary checkpoint
/// slice.  The transaction creates this value only from its own recovery
/// binding, so a complete-looking slice from another operation or plan cannot
/// be accepted as evidence for this one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryDurabilityProof {
    pub(crate) binding: RecoveryBinding,
    content_synchronized: bool,
    metadata_applied: bool,
    renamed: bool,
    parent_synchronized: bool,
}

impl RecoveryDurabilityProof {
    pub(crate) fn from_binding(binding: RecoveryBinding) -> Result<Self, ProofError> {
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

    pub(crate) fn is_complete(&self) -> bool {
        self.content_synchronized
            && self.metadata_applied
            && self.renamed
            && self.parent_synchronized
    }
}

pub(crate) fn ordered_checkpoint(
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
