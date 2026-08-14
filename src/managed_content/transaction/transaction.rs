use super::{
    CleanupDisposition, CleanupResult, Comparison, FailureEvidence, FailureKind, JournalCheckpoint,
    MetadataResult, ProofError, RecoveryBinding, RecoveryDurabilityProof, RecoveryNextAction,
    RecoveryObservation, RecoveryResult, TempArtifact, TempCandidate, TransactionError,
    TransactionPlan, TransactionState,
};

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
