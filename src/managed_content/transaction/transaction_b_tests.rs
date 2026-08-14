use super::transaction_a_tests::{
    artifact, artifact_for, candidate, candidate_for, content_synchronized_state,
    content_written_state, existing_plan, fully_durable_new_recovery_with_candidate,
    metadata_applied_state, plan, renamed_state, temp_created_state,
};
use super::*;
use std::path::PathBuf;

#[test]
fn recovery_proof_cannot_cross_same_operation_with_a_different_plan_identity() {
    let candidate_path = PathBuf::from("config.toml.omnirepo-tmp");
    let source = fully_durable_new_recovery_with_candidate(
        TransactionPlan::new(
            "same-operation",
            PathBuf::from("config.toml"),
            ParentDirectories::existing(),
        )
        .expect("source plan"),
        candidate_path.clone(),
    );
    let proof = source
        .durability_proof()
        .expect("source proof must be complete");

    let different_target = fully_durable_new_recovery_with_candidate(
        TransactionPlan::new(
            "same-operation",
            PathBuf::from("other.toml"),
            ParentDirectories::existing(),
        )
        .expect("different target plan"),
        candidate_path.clone(),
    );
    assert_eq!(
        different_target.clone().finish_recovered_new(proof.clone()),
        Err(TransactionError::DurabilityProofMismatch)
    );

    let created_parents = fully_durable_new_recovery_with_candidate(
        TransactionPlan::new(
            "same-operation",
            PathBuf::from("config.toml"),
            ParentDirectories::created(Vec::<PathBuf>::new()),
        )
        .expect("created-parent variant plan"),
        candidate_path.clone(),
    );
    assert_eq!(
        created_parents.clone().finish_recovered_new(proof.clone()),
        Err(TransactionError::DurabilityProofMismatch)
    );

    let deep_candidate = PathBuf::from("nested/deeper/config.toml.omnirepo-tmp");
    let one_created_parent = fully_durable_new_recovery_with_candidate(
        TransactionPlan::new(
            "same-operation",
            PathBuf::from("nested/deeper/config.toml"),
            ParentDirectories::created(["nested"]),
        )
        .expect("one-parent plan"),
        deep_candidate.clone(),
    );
    let two_created_parents = fully_durable_new_recovery_with_candidate(
        TransactionPlan::new(
            "same-operation",
            PathBuf::from("nested/deeper/config.toml"),
            ParentDirectories::created(["nested", "nested/deeper"]),
        )
        .expect("two-parent plan"),
        deep_candidate,
    );
    let one_parent_proof = one_created_parent
        .durability_proof()
        .expect("one-parent proof must be complete");
    assert_eq!(
        two_created_parents.finish_recovered_new(one_parent_proof),
        Err(TransactionError::DurabilityProofMismatch)
    );
}

#[test]
fn temp_only_recovery_derives_owned_cleanup_without_existing_parent_removal() {
    let recovered = Transaction::begin(existing_plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate_for("config.toml.omnirepo-tmp", 1))
        .expect("reserve")
        .interrupt("temp remained")
        .expect("interrupt")
        .recover(RecoveryObservation::TempOnly(artifact_for(
            "config.toml.omnirepo-tmp",
            1,
            "run-existing",
        )))
        .expect("recover temp-only");

    assert_eq!(
        recovered.cleanup_disposition(),
        CleanupDisposition::TempOnly
    );
    assert_eq!(
        recovered.clone().cleanup(CleanupResult::ParentsRemoved),
        Err(TransactionError::CleanupResultMismatch {
            outstanding: CleanupDisposition::TempOnly,
            reported: CleanupResult::ParentsRemoved,
        })
    );
    assert_eq!(
        recovered
            .cleanup(CleanupResult::TempRemoved)
            .expect("remove only owned temp")
            .cleanup_disposition(),
        CleanupDisposition::None
    );
}

#[test]
fn temp_only_recovery_with_created_parents_authorizes_exact_temp_and_parent_cleanup() {
    let recovered = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .interrupt("exclusive temp created before checkpoint")
        .expect("interrupt")
        .recover(RecoveryObservation::TempOnly(artifact(1)))
        .expect("recover temp-only");

    assert_eq!(
        recovered.cleanup_disposition(),
        CleanupDisposition::TempAndParents
    );
    assert_eq!(
        recovered
            .cleanup(CleanupResult::TempAndParentsRemoved)
            .expect("remove owned temp and created parents")
            .cleanup_disposition(),
        CleanupDisposition::None
    );
}

#[test]
fn temp_only_recovery_rejects_foreign_collision_and_mismatched_artifact() {
    let pending = Transaction::begin(existing_plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate_for("config.toml.omnirepo-tmp", 1))
        .expect("reserve")
        .interrupt("foreign collision observed")
        .expect("interrupt");

    assert_eq!(
        pending
            .clone()
            .recover(RecoveryObservation::TempOnly(artifact_for(
                "config.toml.omnirepo-tmp",
                1,
                "foreign-operation",
            ))),
        Err(TransactionError::ForeignTempOwner)
    );
    assert_eq!(pending.state(), &TransactionState::Interrupted);

    assert_eq!(
        pending
            .clone()
            .recover(RecoveryObservation::TempOnly(artifact_for(
                "other.toml.omnirepo-tmp",
                1,
                "run-existing",
            ))),
        Err(TransactionError::CandidateMismatch)
    );
    assert_eq!(pending.state(), &TransactionState::Interrupted);
}

#[test]
fn new_complete_recovery_never_authorizes_removing_created_parents() {
    let recovered = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .temp_created(artifact(1))
        .expect("create")
        .content_written()
        .expect("write")
        .content_synchronized()
        .expect("sync")
        .metadata_applied(MetadataResult::Preserved)
        .expect("metadata")
        .renamed()
        .expect("rename")
        .parent_synchronized()
        .expect("parent sync")
        .interrupt("target is complete")
        .expect("interrupt")
        .recover(RecoveryObservation::NewComplete)
        .expect("recover");

    assert_eq!(recovered.cleanup_disposition(), CleanupDisposition::None);
    assert_eq!(
        recovered.clone().cleanup(CleanupResult::ParentsRemoved),
        Err(TransactionError::CleanupResultMismatch {
            outstanding: CleanupDisposition::None,
            reported: CleanupResult::ParentsRemoved,
        })
    );
    let recovered = recovered
        .cleanup(CleanupResult::NothingToRemove)
        .expect("complete target has no residue to clean");
    let proof = recovered
        .durability_proof()
        .expect("new complete proof remains required");
    assert_eq!(
        recovered
            .finish_recovered_new(proof)
            .expect("finish recovered new")
            .state(),
        &TransactionState::Synced
    );
}

#[test]
fn invalid_transitions_are_precise_and_do_not_change_the_state() {
    let old = Transaction::begin(plan());
    assert_eq!(
        old.clone().finish_equal(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Old,
            operation: "finish_equal",
        })
    );
    assert_eq!(old.state(), &TransactionState::Old);

    let new = old.compare(Comparison::Different).expect("comparison");
    assert_eq!(
        new.clone().content_written(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::New,
            operation: "content_written",
        })
    );

    let temp = new.reserve_temp(candidate(1)).expect("reserve temp");
    assert_eq!(
        temp.clone().temp_created(artifact(2)),
        Err(TransactionError::CandidateMismatch)
    );
    assert_eq!(temp.state(), &TransactionState::TempCreatedPending);
}

#[test]
fn terminal_and_completed_states_cannot_be_interrupted_or_replayed() {
    let synced = Transaction::begin(plan())
        .compare(Comparison::Equal)
        .expect("compare")
        .finish_equal()
        .expect("sync");
    assert!(matches!(
        synced.clone().interrupt("late"),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Synced,
            operation: "interrupt",
        })
    ));

    let failed = temp_created_state()
        .content_write_failed("write")
        .expect("failure")
        .cleanup(CleanupResult::TempAndParentsRemoved)
        .expect("cleanup")
        .finish_failure()
        .expect("terminal failure");
    assert!(matches!(
        failed.clone().interrupt("late"),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Failed,
            operation: "interrupt",
        })
    ));

    let completed_cleanup = temp_created_state()
        .content_write_failed("write")
        .expect("failure")
        .cleanup(CleanupResult::TempAndParentsRemoved)
        .expect("cleanup");
    assert_eq!(
        completed_cleanup
            .clone()
            .cleanup(CleanupResult::NothingToRemove),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Cleanup,
            operation: "cleanup",
        })
    );
    assert_eq!(
        completed_cleanup.clone().cleanup_failed("late cleanup"),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Cleanup,
            operation: "cleanup_failed",
        })
    );
    assert!(
        completed_cleanup
            .clone()
            .interrupt("late interrupt")
            .is_err()
    );
    let no_failure = renamed_state()
        .parent_synchronized()
        .expect("parent sync")
        .cleanup(CleanupResult::TempConsumed)
        .expect("cleanup");
    assert_eq!(
        no_failure.finish_failure(),
        Err(TransactionError::FailureNotRecorded)
    );
    assert_eq!(
        synced.finish_failure(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Synced,
            operation: "finish_failure",
        })
    );
}

#[test]
fn temp_candidates_must_be_same_directory_and_attempts_must_increase() {
    let other_directory =
        TempCandidate::new(PathBuf::from("elsewhere/config.tmp"), 1).expect("candidate syntax");
    let error = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(other_directory)
        .expect_err("cross-directory temp must be rejected");
    assert_eq!(error, TransactionError::TempOutsideTargetDirectory);

    let transaction = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(2))
        .expect("first candidate");
    assert_eq!(
        transaction.reserve_temp(candidate(1)),
        Err(TransactionError::TempAttemptNotIncreasing)
    );
}

#[test]
fn metadata_failures_interrupt_before_rename_and_require_cleanup() {
    let transaction = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve temp")
        .temp_created(artifact(1))
        .expect("create temp")
        .content_written()
        .expect("write content")
        .content_synchronized()
        .expect("sync content");

    let transaction = transaction
        .metadata_failed("xattr unsupported")
        .expect("record metadata failure");
    assert_eq!(transaction.state(), &TransactionState::Cleanup);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::CleanupRequired);
    assert_eq!(
        transaction.cleanup_disposition(),
        CleanupDisposition::TempAndParents
    );
    assert!(!transaction.was_renamed());
    assert_eq!(transaction.failure_reason(), Some("xattr unsupported"));

    let transaction = transaction
        .cleanup(CleanupResult::TempAndParentsRemoved)
        .expect("clean failed transaction");
    let transaction = transaction
        .finish_failure()
        .expect("terminalize metadata failure");
    assert_eq!(transaction.state(), &TransactionState::Failed);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Failed);
}

#[test]
fn every_stage_failure_records_kind_checkpoint_reason_and_residue() {
    let cases = [
        (
            temp_created_state()
                .content_write_failed("write")
                .expect("write failure"),
            FailureKind::ContentWrite,
            JournalCheckpoint::TempCreated,
            CleanupDisposition::TempAndParents,
        ),
        (
            content_written_state()
                .content_sync_failed("content sync")
                .expect("content sync failure"),
            FailureKind::ContentSynchronization,
            JournalCheckpoint::ContentWritten,
            CleanupDisposition::TempAndParents,
        ),
        (
            content_synchronized_state()
                .metadata_failed("metadata")
                .expect("metadata failure"),
            FailureKind::Metadata,
            JournalCheckpoint::ContentSynchronized,
            CleanupDisposition::TempAndParents,
        ),
        (
            metadata_applied_state()
                .rename_failed("rename")
                .expect("rename failure"),
            FailureKind::Rename,
            JournalCheckpoint::MetadataApplied,
            CleanupDisposition::TempAndParents,
        ),
        (
            renamed_state()
                .parent_sync_failed("parent sync")
                .expect("parent sync failure"),
            FailureKind::ParentSynchronization,
            JournalCheckpoint::Renamed,
            CleanupDisposition::None,
        ),
    ];

    for (transaction, kind, checkpoint, residue) in cases {
        let evidence = transaction.failure().expect("failure evidence");
        assert_eq!(evidence.kind(), kind);
        assert_eq!(evidence.last_durable_checkpoint(), checkpoint);
        assert_eq!(evidence.residue(), residue);
        assert!(!evidence.reason().is_empty());
        assert_eq!(transaction.last_durable_checkpoint(), Some(checkpoint));
        assert_eq!(transaction.state(), &TransactionState::Cleanup);
        assert_eq!(transaction.checkpoint(), JournalCheckpoint::CleanupRequired);
    }
}

#[test]
fn temp_creation_failure_records_typed_io_failure_and_created_parent_residue() {
    let failed = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .temp_creation_failed("exclusive create failed")
        .expect("record temp creation failure");

    let evidence = failed.failure().expect("failure evidence");
    assert_eq!(evidence.kind(), FailureKind::TempCreation);
    assert_eq!(
        evidence.last_durable_checkpoint(),
        JournalCheckpoint::TempCreationStarted
    );
    assert_eq!(evidence.residue(), CleanupDisposition::ParentsOnly);
    assert_eq!(failed.failures().len(), 1);
    assert_eq!(
        failed.cleanup_disposition(),
        CleanupDisposition::ParentsOnly
    );

    let terminal = failed
        .cleanup(CleanupResult::ParentsRemoved)
        .expect("remove only created parents")
        .finish_failure()
        .expect("terminalize temp creation failure");
    assert_eq!(terminal.state(), &TransactionState::Failed);
}

#[test]
fn temp_creation_failure_with_existing_parents_cannot_authorize_parent_removal() {
    let failed = Transaction::begin(existing_plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate_for("config.toml.omnirepo-tmp", 1))
        .expect("reserve")
        .temp_creation_failed("permission denied")
        .expect("record temp creation failure");

    let evidence = failed.failure().expect("failure evidence");
    assert_eq!(evidence.kind(), FailureKind::TempCreation);
    assert_eq!(evidence.residue(), CleanupDisposition::None);
    assert_eq!(failed.cleanup_disposition(), CleanupDisposition::None);
    assert_eq!(
        failed.clone().cleanup(CleanupResult::ParentsRemoved),
        Err(TransactionError::CleanupResultMismatch {
            outstanding: CleanupDisposition::None,
            reported: CleanupResult::ParentsRemoved,
        })
    );
    let terminal = failed
        .cleanup(CleanupResult::NothingToRemove)
        .expect("no owned residue exists")
        .finish_failure()
        .expect("terminalize temp creation failure");
    assert_eq!(terminal.state(), &TransactionState::Failed);
}

#[test]
fn cleanup_failure_is_terminal_and_retains_exact_residue() {
    let transaction = temp_created_state()
        .content_write_failed("write")
        .expect("write failure")
        .cleanup_failed("unable to remove temp");
    let transaction = transaction.expect("cleanup failure transition");
    let evidence = transaction.failure().expect("cleanup evidence");
    assert_eq!(evidence.kind(), FailureKind::Cleanup);
    assert_eq!(
        evidence.last_durable_checkpoint(),
        JournalCheckpoint::CleanupRequired
    );
    assert_eq!(evidence.residue(), CleanupDisposition::TempAndParents);
    assert_eq!(transaction.failures().len(), 2);
    assert_eq!(transaction.failures()[0].kind(), FailureKind::ContentWrite);
    assert_eq!(transaction.failures()[1].kind(), FailureKind::Cleanup);
    assert_eq!(transaction.state(), &TransactionState::Failed);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::CleanupFailed);
    assert_eq!(
        transaction.cleanup_disposition(),
        CleanupDisposition::TempAndParents
    );
    assert!(transaction.clone().finish().is_err());
    assert!(transaction.clone().interrupt("late interrupt").is_err());

    let transaction = temp_created_state()
        .content_write_failed("write")
        .expect("write failure")
        .cleanup_started()
        .expect("start cleanup")
        .cleanup_failed("remove failed")
        .expect("cleanup failure transition");
    assert_eq!(
        transaction
            .failure()
            .expect("cleanup evidence")
            .last_durable_checkpoint(),
        JournalCheckpoint::CleanupStarted
    );
}

#[test]
fn journal_checkpoints_have_stable_replay_labels() {
    let labels: Vec<_> = [
        JournalCheckpoint::Prepared,
        JournalCheckpoint::Compared,
        JournalCheckpoint::TempCreationStarted,
        JournalCheckpoint::TempCollision,
        JournalCheckpoint::TempCreated,
        JournalCheckpoint::ContentWritten,
        JournalCheckpoint::ContentSynchronized,
        JournalCheckpoint::MetadataApplied,
        JournalCheckpoint::Renamed,
        JournalCheckpoint::ParentSynchronized,
        JournalCheckpoint::CleanupStarted,
        JournalCheckpoint::CleanupCompleted,
        JournalCheckpoint::Interrupted,
        JournalCheckpoint::Recovered,
        JournalCheckpoint::CleanupRequired,
        JournalCheckpoint::CleanupFailed,
        JournalCheckpoint::Failed,
        JournalCheckpoint::Synced,
    ]
    .into_iter()
    .map(JournalCheckpoint::label)
    .collect();

    assert_eq!(
        labels,
        vec![
            "prepared",
            "compared",
            "temp-creation-started",
            "temp-collision",
            "temp-created",
            "content-written",
            "content-synchronized",
            "metadata-applied",
            "renamed",
            "parent-synchronized",
            "cleanup-started",
            "cleanup-completed",
            "interrupted",
            "recovered",
            "cleanup-required",
            "cleanup-failed",
            "failed",
            "synced",
        ]
    );
}

#[test]
fn invalid_operation_and_owner_tokens_fail_before_effects() {
    assert_eq!(
        TransactionPlan::new(
            "",
            PathBuf::from("config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::EmptyOperationId)
    );
    assert_eq!(
        TempArtifact::new(candidate(1), ""),
        Err(CandidateError::EmptyOwnerToken)
    );
}

#[test]
fn preflight_faults_are_typed_and_leave_the_old_state_unchanged() {
    let old = Transaction::begin(plan());

    assert_eq!(
        old.durability_proof(),
        Err(ProofError::NotNewCompleteRecovery)
    );
    assert_eq!(
        old.clone().reserve_temp(candidate(1)),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Old,
            operation: "reserve_temp",
        })
    );
    assert_eq!(
        old.clone().cleanup(CleanupResult::NothingToRemove),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Old,
            operation: "cleanup",
        })
    );
    assert_eq!(
        old.clone().cleanup_started(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Old,
            operation: "cleanup_started",
        })
    );
    assert_eq!(old.state(), &TransactionState::Old);

    let collided = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(2))
        .expect("reserve")
        .temp_collision()
        .expect("collision");
    assert_eq!(
        collided.clone().reserve_temp(candidate(1)),
        Err(TransactionError::TempAttemptNotIncreasing)
    );
    assert_eq!(collided.state(), &TransactionState::TempCollision);
}

#[test]
fn cleanup_and_finish_faults_require_a_durable_cleanup_checkpoint() {
    let completed = renamed_state()
        .parent_synchronized()
        .expect("parent synchronization")
        .cleanup(CleanupResult::TempConsumed)
        .expect("cleanup");
    assert_eq!(
        completed.clone().cleanup_started(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Cleanup,
            operation: "cleanup_started",
        })
    );

    let started = renamed_state()
        .parent_synchronized()
        .expect("parent synchronization")
        .cleanup_started()
        .expect("start cleanup");
    assert_eq!(
        started.clone().finish(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Cleanup,
            operation: "finish",
        })
    );
    assert_eq!(
        started.finish_failure(),
        Err(TransactionError::InvalidTransition {
            from: TransactionState::Cleanup,
            operation: "finish_failure",
        })
    );
}
