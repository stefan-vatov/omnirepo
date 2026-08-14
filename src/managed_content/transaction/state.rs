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
