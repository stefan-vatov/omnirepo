//! Pure state for immutable source snapshots.
//!
//! This module deliberately does not acquire a source, access Git, or touch a
//! filesystem.  Those effects belong to adapters.  The types here make the
//! boundary between an adapter and the lifecycle explicit: an adapter owns a
//! lease, prepares a complete staged snapshot, and asks this state machine to
//! publish it atomically.

use std::{error::Error, fmt};

/// A stable source declaration identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(String);

impl SourceId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::Empty { field: "source id" });
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        }) {
            return Err(IdentityError::InvalidSourceId { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! opaque_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(IdentityError::Empty { field: $field });
                }
                if value.as_bytes().contains(&0) {
                    return Err(IdentityError::ContainsNul { field: $field });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

opaque_id!(RevisionId, "revision");
opaque_id!(SnapshotId, "snapshot id");
opaque_id!(OperationId, "operation id");
opaque_id!(StagingId, "staging id");
opaque_id!(CacheKey, "cache key");

/// Errors raised while constructing immutable identity values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityError {
    Empty { field: &'static str },
    ContainsNul { field: &'static str },
    InvalidSourceId { value: String },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { field } => write!(formatter, "{field} must not be empty"),
            Self::ContainsNul { field } => write!(formatter, "{field} must not contain NUL"),
            Self::InvalidSourceId { value } => write!(
                formatter,
                "source id {value:?} must contain only lowercase ASCII letters, digits, '.', '_', or '-'"
            ),
        }
    }
}

impl Error for IdentityError {}

/// The canonical identity of one configured source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceIdentity {
    id: SourceId,
    locator: String,
}

impl SourceIdentity {
    pub fn new(id: SourceId, locator: impl Into<String>) -> Result<Self, IdentityError> {
        let locator = locator.into();
        if locator.is_empty() {
            return Err(IdentityError::Empty {
                field: "source locator",
            });
        }
        if locator.as_bytes().contains(&0) {
            return Err(IdentityError::ContainsNul {
                field: "source locator",
            });
        }
        Ok(Self { id, locator })
    }

    pub fn id(&self) -> &SourceId {
        &self.id
    }

    pub fn locator(&self) -> &str {
        &self.locator
    }
}

/// The freshness observed for an already published immutable snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    Fresh,
    Stale,
}

/// An immutable, complete source snapshot and the authority that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedSnapshot {
    source: SourceIdentity,
    revision: RevisionId,
    snapshot_id: SnapshotId,
    cache: CacheKey,
}

impl PublishedSnapshot {
    pub fn new(
        source: SourceIdentity,
        revision: RevisionId,
        snapshot_id: SnapshotId,
        cache: CacheKey,
    ) -> Self {
        Self {
            source,
            revision,
            snapshot_id,
            cache,
        }
    }

    pub fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub fn revision(&self) -> &RevisionId {
        &self.revision
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn cache(&self) -> &CacheKey {
        &self.cache
    }
}

/// The exclusive ownership token for one source materialization attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationLease {
    source: SourceIdentity,
    operation: OperationId,
    staging: StagingId,
    cache: CacheKey,
}

impl MaterializationLease {
    pub fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }

    pub fn staging(&self) -> &StagingId {
        &self.staging
    }

    pub fn cache(&self) -> &CacheKey {
        &self.cache
    }

    #[cfg(test)]
    pub fn test_fixture(operation: impl Into<String>) -> Self {
        Self {
            source: SourceIdentity {
                id: SourceId("fixture".to_owned()),
                locator: "fixture".to_owned(),
            },
            operation: OperationId(operation.into()),
            staging: StagingId("fixture-staging".to_owned()),
            cache: CacheKey("fixture-cache".to_owned()),
        }
    }
}

/// Cleanup evidence attached to a failed or interrupted materialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupOutcome {
    Removed,
    RetainedForRecovery,
    NothingToRemove,
    Failed { reason: String },
}

impl CleanupOutcome {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::RetainedForRecovery => "retained-for-recovery",
            Self::NothingToRemove => "nothing-to-remove",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Failure categories are facts from the acquisition or publication adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    FetchFailed,
    CacheCorrupt,
    PublicationFailed,
    CleanupFailed,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FetchFailed => "fetch-failed",
            Self::CacheCorrupt => "cache-corrupt",
            Self::PublicationFailed => "publication-failed",
            Self::CleanupFailed => "cleanup-failed",
        }
    }
}

/// Durable facts from attempts that need recovery.
///
/// Recovery must not erase the causal evidence that caused it.  The lifecycle
/// can persist this value in its run journal while a later attempt proceeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryEvidence {
    Failed {
        operation: OperationId,
        kind: FailureKind,
        message: String,
        cleanup: CleanupOutcome,
    },
    Interrupted {
        operation: OperationId,
        staging: StagingId,
        reason: String,
        cleanup: CleanupOutcome,
    },
}

/// A complete state of one source's snapshot store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotState {
    Absent,
    Fresh(PublishedSnapshot),
    Stale(PublishedSnapshot),
    InProgress {
        lease: MaterializationLease,
        previous: Option<PublishedSnapshot>,
        previous_freshness: Option<Freshness>,
    },
    Complete(PublishedSnapshot),
    Failed {
        operation: OperationId,
        kind: FailureKind,
        message: String,
        cleanup: CleanupOutcome,
        previous: Option<PublishedSnapshot>,
        previous_freshness: Option<Freshness>,
    },
    Interrupted {
        operation: OperationId,
        staging: StagingId,
        reason: String,
        cleanup: CleanupOutcome,
        previous: Option<PublishedSnapshot>,
        previous_freshness: Option<Freshness>,
    },
}

/// Invalid state-machine operations are explicit and deterministic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionError {
    ConcurrentMaterializer,
    NotInProgress,
    LeaseMismatch,
    SourceIdentityMismatch,
    CacheMismatch,
    NoPublishedSnapshot,
    RecoveryRequired,
    InterruptedStagingMustBeRetained,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ConcurrentMaterializer => "a materializer already owns this source",
            Self::NotInProgress => "the snapshot is not being materialized",
            Self::LeaseMismatch => "the materialization lease is not current",
            Self::SourceIdentityMismatch => "the staged snapshot belongs to another source",
            Self::CacheMismatch => "the staged snapshot belongs to another cache entry",
            Self::NoPublishedSnapshot => "no published snapshot is available",
            Self::RecoveryRequired => {
                "the previous attempt must be recovered explicitly before materialization"
            }
            Self::InterruptedStagingMustBeRetained => {
                "interrupted staging must be retained for recovery"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for TransitionError {}

/// A reader is a stable view of one immutable publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotReader {
    snapshot: PublishedSnapshot,
}

impl SnapshotReader {
    pub fn snapshot(&self) -> &PublishedSnapshot {
        &self.snapshot
    }
}

/// Pure lifecycle state for one canonical source identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotStore {
    source: SourceIdentity,
    state: SnapshotState,
    recovery_evidence: Vec<RecoveryEvidence>,
}

impl SnapshotStore {
    pub fn new(source: SourceIdentity) -> Self {
        Self {
            source,
            state: SnapshotState::Absent,
            recovery_evidence: Vec::new(),
        }
    }

    pub fn with_published(
        source: SourceIdentity,
        snapshot: PublishedSnapshot,
        freshness: Freshness,
    ) -> Result<Self, TransitionError> {
        if snapshot.source != source {
            return Err(TransitionError::SourceIdentityMismatch);
        }
        let state = match freshness {
            Freshness::Fresh => SnapshotState::Fresh(snapshot),
            Freshness::Stale => SnapshotState::Stale(snapshot),
        };
        Ok(Self {
            source,
            state,
            recovery_evidence: Vec::new(),
        })
    }

    pub fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub fn state(&self) -> &SnapshotState {
        &self.state
    }

    pub fn recovery_evidence(&self) -> &[RecoveryEvidence] {
        &self.recovery_evidence
    }

    /// Return the last complete publication for diagnostics and recovery.
    ///
    /// This accessor does not authorize a source read. It may return a stale
    /// or superseded publication while a refresh, failure, or interruption is
    /// being recorded.
    pub fn published(&self) -> Option<&PublishedSnapshot> {
        match &self.state {
            SnapshotState::Absent => None,
            SnapshotState::Fresh(snapshot)
            | SnapshotState::Stale(snapshot)
            | SnapshotState::Complete(snapshot) => Some(snapshot),
            SnapshotState::InProgress { previous, .. }
            | SnapshotState::Failed { previous, .. }
            | SnapshotState::Interrupted { previous, .. } => previous.as_ref(),
        }
    }

    /// Return the current publication only when it is authoritative.
    ///
    /// A previous publication remains available through
    /// [`Self::diagnostic_snapshot`] while a materializer is in progress or a
    /// recovery state is retained. It must not be used as current authority.
    pub fn reader(&self) -> Option<SnapshotReader> {
        let snapshot = match &self.state {
            SnapshotState::Fresh(snapshot) | SnapshotState::Complete(snapshot) => Some(snapshot),
            SnapshotState::Absent
            | SnapshotState::Stale(_)
            | SnapshotState::InProgress { .. }
            | SnapshotState::Failed { .. }
            | SnapshotState::Interrupted { .. } => None,
        };
        snapshot
            .cloned()
            .map(|snapshot| SnapshotReader { snapshot })
    }

    /// Return the latest publication for evidence, not authority.
    pub fn diagnostic_snapshot(&self) -> Option<&PublishedSnapshot> {
        self.published()
    }

    pub fn begin(
        &mut self,
        operation: OperationId,
        staging: StagingId,
        cache: CacheKey,
    ) -> Result<MaterializationLease, TransitionError> {
        if matches!(self.state, SnapshotState::InProgress { .. }) {
            return Err(TransitionError::ConcurrentMaterializer);
        }
        if matches!(
            self.state,
            SnapshotState::Failed { .. } | SnapshotState::Interrupted { .. }
        ) {
            return Err(TransitionError::RecoveryRequired);
        }
        self.begin_materialization(operation, staging, cache)
    }

    /// Explicitly resume a failed or interrupted attempt.
    ///
    /// This separate transition makes recovery visible to the caller and
    /// leaves all prior [`RecoveryEvidence`] in the store.
    pub fn recover(
        &mut self,
        operation: OperationId,
        staging: StagingId,
        cache: CacheKey,
    ) -> Result<MaterializationLease, TransitionError> {
        if !matches!(
            self.state,
            SnapshotState::Failed { .. } | SnapshotState::Interrupted { .. }
        ) {
            return Err(TransitionError::NoPublishedSnapshot);
        }
        self.begin_materialization(operation, staging, cache)
    }

    fn begin_materialization(
        &mut self,
        operation: OperationId,
        staging: StagingId,
        cache: CacheKey,
    ) -> Result<MaterializationLease, TransitionError> {
        let lease = MaterializationLease {
            source: self.source.clone(),
            operation,
            staging,
            cache,
        };
        let (previous, previous_freshness) = self.previous_snapshot();
        self.state = SnapshotState::InProgress {
            lease: lease.clone(),
            previous,
            previous_freshness,
        };
        Ok(lease)
    }

    fn previous_snapshot(&self) -> (Option<PublishedSnapshot>, Option<Freshness>) {
        match &self.state {
            SnapshotState::Absent => (None, None),
            SnapshotState::Fresh(snapshot) => (Some(snapshot.clone()), Some(Freshness::Fresh)),
            SnapshotState::Stale(snapshot) => (Some(snapshot.clone()), Some(Freshness::Stale)),
            SnapshotState::Complete(snapshot) => (Some(snapshot.clone()), Some(Freshness::Fresh)),
            SnapshotState::InProgress {
                previous,
                previous_freshness,
                ..
            }
            | SnapshotState::Failed {
                previous,
                previous_freshness,
                ..
            }
            | SnapshotState::Interrupted {
                previous,
                previous_freshness,
                ..
            } => (previous.clone(), *previous_freshness),
        }
    }

    /// Publish a complete staged snapshot as one state transition.
    ///
    /// Readers see the previous complete publication until this method
    /// succeeds.  They never observe the in-progress staging identity.
    pub fn publish(
        &mut self,
        lease: &MaterializationLease,
        snapshot: PublishedSnapshot,
    ) -> Result<(), TransitionError> {
        let current = match &self.state {
            SnapshotState::InProgress { lease, .. } => lease,
            _ => return Err(TransitionError::NotInProgress),
        };
        if current != lease {
            return Err(TransitionError::LeaseMismatch);
        }
        if snapshot.source != self.source {
            return Err(TransitionError::SourceIdentityMismatch);
        }
        if snapshot.cache != lease.cache {
            return Err(TransitionError::CacheMismatch);
        }
        self.state = SnapshotState::Complete(snapshot);
        Ok(())
    }

    pub fn classify(&mut self, freshness: Freshness) -> Result<(), TransitionError> {
        let snapshot = match &self.state {
            SnapshotState::Fresh(snapshot)
            | SnapshotState::Stale(snapshot)
            | SnapshotState::Complete(snapshot) => snapshot.clone(),
            _ => return Err(TransitionError::NoPublishedSnapshot),
        };
        self.state = match freshness {
            Freshness::Fresh => SnapshotState::Fresh(snapshot),
            Freshness::Stale => SnapshotState::Stale(snapshot),
        };
        Ok(())
    }

    pub fn fail(
        &mut self,
        lease: &MaterializationLease,
        kind: FailureKind,
        message: impl Into<String>,
        cleanup: CleanupOutcome,
    ) -> Result<CleanupOutcome, TransitionError> {
        let (current, previous, previous_freshness) = match &self.state {
            SnapshotState::InProgress {
                lease: current,
                previous,
                previous_freshness,
            } => (current.clone(), previous.clone(), *previous_freshness),
            _ => return Err(TransitionError::NotInProgress),
        };
        if &current != lease {
            return Err(TransitionError::LeaseMismatch);
        }
        let message = message.into();
        self.recovery_evidence.push(RecoveryEvidence::Failed {
            operation: lease.operation.clone(),
            kind,
            message: message.clone(),
            cleanup: cleanup.clone(),
        });
        self.state = SnapshotState::Failed {
            operation: lease.operation.clone(),
            kind,
            message,
            cleanup: cleanup.clone(),
            previous,
            previous_freshness,
        };
        Ok(cleanup)
    }

    pub fn interrupt(
        &mut self,
        lease: &MaterializationLease,
        reason: impl Into<String>,
        cleanup: CleanupOutcome,
    ) -> Result<CleanupOutcome, TransitionError> {
        if cleanup != CleanupOutcome::RetainedForRecovery {
            return Err(TransitionError::InterruptedStagingMustBeRetained);
        }
        let (current, previous, previous_freshness) = match &self.state {
            SnapshotState::InProgress {
                lease: current,
                previous,
                previous_freshness,
            } => (current.clone(), previous.clone(), *previous_freshness),
            _ => return Err(TransitionError::NotInProgress),
        };
        if &current != lease {
            return Err(TransitionError::LeaseMismatch);
        }
        let reason = reason.into();
        self.recovery_evidence.push(RecoveryEvidence::Interrupted {
            operation: lease.operation.clone(),
            staging: lease.staging.clone(),
            reason: reason.clone(),
            cleanup: cleanup.clone(),
        });
        self.state = SnapshotState::Interrupted {
            operation: lease.operation.clone(),
            staging: lease.staging.clone(),
            reason,
            cleanup: cleanup.clone(),
            previous,
            previous_freshness,
        };
        Ok(cleanup)
    }

    /// A stable, bounded representation suitable for journal replay logs.
    pub fn replay_label(&self) -> String {
        match &self.state {
            SnapshotState::Absent => "state=absent".to_owned(),
            SnapshotState::Fresh(snapshot) => {
                format!("state=fresh;revision={}", snapshot.revision.as_str())
            }
            SnapshotState::Stale(snapshot) => {
                format!("state=stale;revision={}", snapshot.revision.as_str())
            }
            SnapshotState::InProgress { lease, .. } => format!(
                "state=in-progress;operation={};staging={}",
                lease.operation.as_str(),
                lease.staging.as_str()
            ),
            SnapshotState::Complete(snapshot) => {
                format!("state=complete;revision={}", snapshot.revision.as_str())
            }
            SnapshotState::Failed { kind, cleanup, .. } => format!(
                "state=failed;kind={};cleanup={}",
                kind.as_str(),
                cleanup.label()
            ),
            SnapshotState::Interrupted {
                operation,
                staging,
                cleanup,
                ..
            } => format!(
                "state=interrupted;operation={};staging={};cleanup={}",
                operation.as_str(),
                staging.as_str(),
                cleanup.label()
            ),
        }
    }
}
