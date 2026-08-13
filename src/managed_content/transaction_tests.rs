//! Pure transition coverage for the managed-file replacement protocol.
//!
//! This suite deliberately does not touch the filesystem.  The transaction
//! model records the effects that the filesystem implementation must perform;
//! the later integration leaf can bind those states to real operations.

use std::path::PathBuf;

use super::transaction::*;
/*
    CandidateError, CleanupDisposition, CleanupResult, Comparison, ContentVisibility, FailureKind,
    JournalCheckpoint, MetadataResult, ParentDirectories, PlanError, ProofError,
    RecoveryNextAction, RecoveryObservation, RecoveryResult, TempArtifact, TempCandidate,
    Transaction, TransactionError, TransactionPlan, TransactionState,
*/

fn plan() -> TransactionPlan {
    TransactionPlan::new(
        "run-001",
        PathBuf::from("nested/config.toml"),
        ParentDirectories::created(["nested"]),
    )
    .expect("valid transaction plan")
}

fn candidate(attempt: u32) -> TempCandidate {
    TempCandidate::new(PathBuf::from("nested/config.toml.omnirepo-tmp"), attempt)
        .expect("valid same-directory candidate")
}

fn artifact(attempt: u32) -> TempArtifact {
    TempArtifact::new(candidate(attempt), "run-001").expect("valid owned temporary artifact")
}

fn existing_plan() -> TransactionPlan {
    TransactionPlan::new(
        "run-existing",
        PathBuf::from("config.toml"),
        ParentDirectories::existing(),
    )
    .expect("valid existing-parent plan")
}

fn durable_plan(operation_id: &str) -> TransactionPlan {
    TransactionPlan::new(
        operation_id,
        PathBuf::from("config.toml"),
        ParentDirectories::existing(),
    )
    .expect("valid durable recovery plan")
}

fn candidate_for(path: &str, attempt: u32) -> TempCandidate {
    TempCandidate::new(PathBuf::from(path), attempt).expect("valid candidate")
}

fn artifact_for(path: &str, attempt: u32, owner: &str) -> TempArtifact {
    TempArtifact::new(candidate_for(path, attempt), owner).expect("valid artifact")
}

fn temp_created_state() -> Transaction {
    Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .temp_created(artifact(1))
        .expect("create")
}

fn content_written_state() -> Transaction {
    temp_created_state()
        .content_written()
        .expect("write content")
}

fn content_synchronized_state() -> Transaction {
    content_written_state()
        .content_synchronized()
        .expect("sync content")
}

fn metadata_applied_state() -> Transaction {
    content_synchronized_state()
        .metadata_applied(MetadataResult::Preserved)
        .expect("apply metadata")
}

fn renamed_state() -> Transaction {
    metadata_applied_state().renamed().expect("rename")
}

fn fully_durable_new_recovery(plan: TransactionPlan) -> Transaction {
    fully_durable_new_recovery_with_candidate(plan, PathBuf::from("config.toml.omnirepo-tmp"))
}

fn fully_durable_new_recovery_with_candidate(
    plan: TransactionPlan,
    candidate_path: PathBuf,
) -> Transaction {
    let operation_id = plan.operation_id().to_owned();
    let candidate = TempCandidate::new(candidate_path, 1).expect("candidate");
    Transaction::begin(plan)
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate.clone())
        .expect("reserve")
        .temp_created(TempArtifact::new(candidate, operation_id).expect("artifact"))
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
        .interrupt("new complete")
        .expect("interrupt")
        .recover(RecoveryObservation::NewComplete)
        .expect("recover")
        .cleanup(CleanupResult::NothingToRemove)
        .expect("cleanup")
}

#[test]
fn plans_reject_unsafe_or_ambiguous_lexical_paths() {
    assert_eq!(
        TransactionPlan::new("run", PathBuf::new(), ParentDirectories::existing()),
        Err(PlanError::EmptyPath { field: "target" })
    );
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("/tmp/config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::AbsolutePath {
            field: "target",
            ..
        })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("nested/../config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::ParentTraversal {
            field: "target",
            ..
        })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("./config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::CurrentDirectoryComponent {
            field: "target",
            ..
        })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("nested//config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::EmptyComponent {
            field: "target",
            ..
        })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("nested\\config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::InvalidSeparator {
            field: "target",
            ..
        })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("C:/config.toml"),
            ParentDirectories::existing()
        ),
        Err(PlanError::WindowsPrefix {
            field: "target",
            ..
        })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("nested/config.toml"),
            ParentDirectories::created(["nested", "nested"])
        ),
        Err(PlanError::DuplicateParent { .. })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("nested/config.toml"),
            ParentDirectories::created(["other"])
        ),
        Err(PlanError::ParentOutsideTarget { .. })
    ));
    assert!(matches!(
        TransactionPlan::new(
            "run",
            PathBuf::from("nested/config.toml"),
            ParentDirectories::created(["nested/config.toml"])
        ),
        Err(PlanError::ParentOutsideTarget { .. })
    ));
}

#[test]
fn temporary_candidates_reject_unsafe_paths_and_zero_attempts() {
    assert_eq!(
        TempCandidate::new(PathBuf::new(), 1),
        Err(CandidateError::EmptyPath)
    );
    assert_eq!(
        TempCandidate::new(PathBuf::from("nested/../temp"), 1),
        Err(CandidateError::InvalidPath(PlanError::ParentTraversal {
            field: "temporary candidate",
            path: "nested/../temp".to_owned(),
        }))
    );
    assert_eq!(
        TempCandidate::new(PathBuf::from("temp"), 0),
        Err(CandidateError::ZeroAttempt)
    );
}

#[test]
fn plan_and_artifact_identities_are_explicit_and_parent_policy_is_typed() {
    let parents = ParentDirectories::existing();
    let plan = TransactionPlan::new(
        "run-existing",
        PathBuf::from("config.toml"),
        parents.clone(),
    )
    .expect("valid existing-parent plan");
    assert_eq!(plan.operation_id(), "run-existing");
    assert_eq!(plan.target(), PathBuf::from("config.toml"));
    assert_eq!(plan.parents(), &parents);

    let candidate =
        TempCandidate::new(PathBuf::from("config.toml.omnirepo-tmp"), 1).expect("candidate");
    let artifact = TempArtifact::new(candidate.clone(), "run-existing").expect("artifact");
    assert_eq!(artifact.candidate(), &candidate);
    assert_eq!(artifact.owner_token(), "run-existing");

    let transaction = Transaction::begin(plan)
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate)
        .expect("reserve")
        .temp_created(artifact)
        .expect("create")
        .content_written()
        .expect("write")
        .content_synchronized()
        .expect("sync")
        .metadata_applied(MetadataResult::NotRequired)
        .expect("metadata")
        .renamed()
        .expect("rename")
        .parent_synchronized()
        .expect("parent sync")
        .cleanup(CleanupResult::TempConsumed)
        .expect("cleanup")
        .finish()
        .expect("finish");
    assert_eq!(transaction.state(), &TransactionState::Synced);

    // Keep the two distinct result names visible to callers that need to
    // account for a parent-only cleanup after recovery.
    assert_eq!(CleanupResult::ParentsRemoved, CleanupResult::ParentsRemoved);
    assert_eq!(MetadataResult::Preserved, MetadataResult::Preserved);
}

#[test]
fn initial_old_state_has_named_prepared_checkpoint_and_parent_cleanup_obligation() {
    let transaction = Transaction::begin(plan());

    assert_eq!(transaction.state(), &TransactionState::Old);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Prepared);
    assert_eq!(
        transaction.cleanup_disposition(),
        CleanupDisposition::ParentsOnly
    );
    assert_eq!(transaction.journal_labels(), ["prepared"]);
}

#[test]
fn equal_content_finishes_as_a_true_noop_without_filesystem_effects() {
    let transaction = Transaction::begin(plan())
        .compare(Comparison::Equal)
        .expect("comparison");

    assert_eq!(transaction.state(), &TransactionState::Equal);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Compared);

    let transaction = transaction.finish_equal().expect("equal no-op");
    assert_eq!(transaction.state(), &TransactionState::Synced);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Synced);
    assert_eq!(transaction.cleanup_disposition(), CleanupDisposition::None);
    assert_eq!(
        transaction.cleanup_result(),
        Some(CleanupResult::NothingToRemove)
    );
    assert_eq!(
        transaction.journal_labels(),
        ["prepared", "compared", "synced"]
    );
}

#[test]
fn new_content_moves_through_every_durable_replacement_checkpoint() {
    let transaction = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison");
    assert_eq!(transaction.state(), &TransactionState::New);

    let transaction = transaction
        .reserve_temp(candidate(1))
        .expect("reserve temp");
    assert_eq!(
        transaction.checkpoint(),
        JournalCheckpoint::TempCreationStarted
    );

    let transaction = transaction.temp_created(artifact(1)).expect("create temp");
    assert_eq!(transaction.state(), &TransactionState::TempCreated);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::TempCreated);

    let transaction = transaction.content_written().expect("write content");
    assert_eq!(transaction.state(), &TransactionState::ContentWritten);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::ContentWritten);

    let transaction = transaction.content_synchronized().expect("sync temp");
    assert_eq!(
        transaction.checkpoint(),
        JournalCheckpoint::ContentSynchronized
    );

    let transaction = transaction
        .metadata_applied(MetadataResult::Preserved)
        .expect("apply metadata");
    assert_eq!(transaction.state(), &TransactionState::MetadataApplied);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::MetadataApplied);

    let transaction = transaction.renamed().expect("rename atomically");
    assert_eq!(transaction.state(), &TransactionState::Renamed);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Renamed);

    let transaction = transaction.parent_synchronized().expect("sync parent");
    assert_eq!(
        transaction.checkpoint(),
        JournalCheckpoint::ParentSynchronized
    );

    let transaction = transaction
        .cleanup(CleanupResult::TempConsumed)
        .expect("cleanup");
    assert_eq!(transaction.state(), &TransactionState::Cleanup);
    assert_eq!(
        transaction.checkpoint(),
        JournalCheckpoint::CleanupCompleted
    );

    let transaction = transaction.finish().expect("finish replacement");
    assert_eq!(transaction.state(), &TransactionState::Synced);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Synced);
    assert_eq!(transaction.cleanup_disposition(), CleanupDisposition::None);
    assert_eq!(
        transaction.journal_labels(),
        [
            "prepared",
            "compared",
            "temp-creation-started",
            "temp-created",
            "content-written",
            "content-synchronized",
            "metadata-applied",
            "renamed",
            "parent-synchronized",
            "cleanup-started",
            "cleanup-completed",
            "synced",
        ]
    );
}

#[test]
fn colliding_temp_names_are_recorded_and_never_become_owned_residue() {
    let transaction = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve temp")
        .temp_collision()
        .expect("record collision");

    assert_eq!(transaction.state(), &TransactionState::TempCollision);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::TempCollision);
    assert_eq!(transaction.collisions(), 1);
    assert_eq!(transaction.collision_candidates()[0].attempt(), 1);
    assert_eq!(
        transaction.cleanup_disposition(),
        CleanupDisposition::ParentsOnly
    );

    let transaction = transaction
        .reserve_temp(candidate(2))
        .expect("try a fresh candidate")
        .temp_created(artifact(2))
        .expect("create owned temp");
    assert_eq!(transaction.state(), &TransactionState::TempCreated);
    assert_eq!(transaction.collisions(), 1);
}

#[test]
fn temp_ownership_requires_the_plan_operation_and_exact_candidate_attempt() {
    let pending = Transaction::begin(existing_plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate_for("config.toml.omnirepo-tmp", 1))
        .expect("reserve temp");

    assert_eq!(
        pending
            .clone()
            .temp_created(artifact_for("config.toml.omnirepo-tmp", 2, "run-existing")),
        Err(TransactionError::CandidateMismatch)
    );
    assert_eq!(
        pending
            .clone()
            .temp_created(artifact_for("config.toml.omnirepo-tmp", 1, "another-run")),
        Err(TransactionError::ForeignTempOwner)
    );
    let owned = pending
        .temp_created(artifact_for("config.toml.omnirepo-tmp", 1, "run-existing"))
        .expect("matching operation owns the temp");
    assert_eq!(owned.cleanup_disposition(), CleanupDisposition::TempOnly);
}

#[test]
fn cleanup_results_must_match_exact_residue_and_existing_parents_are_never_removed() {
    let created = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .temp_created(artifact(1))
        .expect("create");
    let failed = created
        .content_write_failed("write failed")
        .expect("record write failure");
    assert_eq!(
        failed.clone().cleanup(CleanupResult::TempRemoved),
        Err(TransactionError::CleanupResultMismatch {
            outstanding: CleanupDisposition::TempAndParents,
            reported: CleanupResult::TempRemoved,
        })
    );
    let cleaned = failed
        .cleanup(CleanupResult::TempAndParentsRemoved)
        .expect("remove temp and created parents");
    assert_eq!(cleaned.cleanup_disposition(), CleanupDisposition::None);

    let existing = Transaction::begin(existing_plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate_for("config.toml.omnirepo-tmp", 1))
        .expect("reserve")
        .temp_created(artifact_for("config.toml.omnirepo-tmp", 1, "run-existing"))
        .expect("create")
        .content_write_failed("write failed")
        .expect("record failure");
    assert_eq!(existing.cleanup_disposition(), CleanupDisposition::TempOnly);
    assert_eq!(
        existing.clone().cleanup(CleanupResult::ParentsRemoved),
        Err(TransactionError::CleanupResultMismatch {
            outstanding: CleanupDisposition::TempOnly,
            reported: CleanupResult::ParentsRemoved,
        })
    );
    assert!(
        existing
            .cleanup(CleanupResult::TempRemoved)
            .expect("remove only owned temp")
            .cleanup_result()
            .is_some()
    );
}

#[test]
fn interruption_records_the_last_durable_checkpoint_and_owned_residue() {
    let transaction = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve temp")
        .temp_created(artifact(1))
        .expect("create temp")
        .content_written()
        .expect("write content");

    let transaction = transaction
        .interrupt("process terminated")
        .expect("interrupt");
    assert_eq!(transaction.state(), &TransactionState::Interrupted);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Interrupted);
    assert_eq!(
        transaction.interrupted_at(),
        Some(JournalCheckpoint::ContentWritten)
    );
    assert_eq!(
        transaction.cleanup_disposition(),
        CleanupDisposition::TempAndParents
    );

    let transaction = transaction
        .recover(RecoveryObservation::OldComplete)
        .expect("recover old complete target");
    assert_eq!(transaction.state(), &TransactionState::Recovered);
    assert_eq!(transaction.checkpoint(), JournalCheckpoint::Recovered);
    assert_eq!(
        transaction.recovery_result(),
        Some(RecoveryResult::OldComplete)
    );

    let transaction = transaction
        .cleanup(CleanupResult::TempAndParentsRemoved)
        .expect("remove only owned residue");
    assert_eq!(transaction.state(), &TransactionState::Cleanup);
    assert_eq!(transaction.cleanup_disposition(), CleanupDisposition::None);
    assert!(transaction.clone().finish().is_err());
}

#[test]
fn recovery_observations_are_typed_and_preserve_old_or_new_complete_visibility() {
    for (observation, action, residue) in [
        (
            RecoveryObservation::OldComplete,
            RecoveryNextAction::RetryReplacement,
            CleanupDisposition::TempAndParents,
        ),
        (
            RecoveryObservation::NewComplete,
            RecoveryNextAction::ProveNewDurability,
            CleanupDisposition::None,
        ),
        (
            RecoveryObservation::TempOnly(artifact(1)),
            RecoveryNextAction::CleanupOwnedResidue,
            CleanupDisposition::TempAndParents,
        ),
        (
            RecoveryObservation::Unknown,
            RecoveryNextAction::Investigate,
            CleanupDisposition::TempAndParents,
        ),
    ] {
        let transaction = Transaction::begin(plan())
            .compare(Comparison::Different)
            .expect("comparison")
            .reserve_temp(candidate(1))
            .expect("reserve temp")
            .temp_created(artifact(1))
            .expect("create temp")
            .interrupt("cancelled")
            .expect("interrupt")
            .recover(observation)
            .expect("recovery observation");
        assert_eq!(transaction.state(), &TransactionState::Recovered);
        assert_eq!(transaction.checkpoint(), JournalCheckpoint::Recovered);
        assert_eq!(transaction.recovery_action(), Some(action));
        assert_eq!(transaction.cleanup_disposition(), residue);
    }

    assert_eq!(ContentVisibility::Old.label(), "old-complete");
    assert_eq!(ContentVisibility::New.label(), "new-complete");
    assert_eq!(ContentVisibility::Equal.label(), "equal-noop");
}

#[test]
fn only_new_complete_recovery_can_finish_and_only_after_the_durability_proof() {
    let incomplete = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .temp_created(artifact(1))
        .expect("create")
        .interrupt("incomplete new complete")
        .expect("interrupt")
        .recover(RecoveryObservation::NewComplete)
        .expect("recover")
        .cleanup(CleanupResult::NothingToRemove)
        .expect("cleanup");
    assert_eq!(incomplete.durability_proof(), Err(ProofError::Incomplete));

    let fully_durable = fully_durable_new_recovery(durable_plan("run-durable"));
    let proof = fully_durable
        .durability_proof()
        .expect("proof must be derived from the exact recovered journal");

    let old = Transaction::begin(plan())
        .compare(Comparison::Different)
        .expect("comparison")
        .reserve_temp(candidate(1))
        .expect("reserve")
        .temp_created(artifact(1))
        .expect("create")
        .interrupt("old complete")
        .expect("interrupt")
        .recover(RecoveryObservation::OldComplete)
        .expect("recover")
        .cleanup(CleanupResult::TempAndParentsRemoved)
        .expect("cleanup");
    assert_eq!(
        old.clone().finish_recovered_new(proof.clone()),
        Err(TransactionError::DurabilityProofRequired)
    );
    assert!(old.finish().is_err());

    let new = fully_durable_new_recovery(durable_plan("run-durable"));
    assert_eq!(new.cleanup_disposition(), CleanupDisposition::None);
    let new = new
        .finish_recovered_new(proof)
        .expect("finish only after proof");
    assert_eq!(new.state(), &TransactionState::Synced);
    assert_eq!(new.recovery_result(), Some(RecoveryResult::NewComplete));
    assert_eq!(
        new.recovery_action(),
        Some(RecoveryNextAction::ProveNewDurability)
    );

    for observation in [
        RecoveryObservation::TempOnly(artifact(1)),
        RecoveryObservation::Unknown,
    ] {
        let transaction = Transaction::begin(plan())
            .compare(Comparison::Different)
            .expect("comparison")
            .reserve_temp(candidate(1))
            .expect("reserve")
            .temp_created(artifact(1))
            .expect("create")
            .interrupt("not complete")
            .expect("interrupt")
            .recover(observation)
            .expect("recover")
            .cleanup(CleanupResult::TempAndParentsRemoved)
            .expect("cleanup");
        assert!(transaction.clone().finish().is_err());
        assert_eq!(transaction.state(), &TransactionState::Cleanup);
    }
}

#[test]
fn recovery_proof_cannot_cross_operation_or_journal_identity() {
    let source = fully_durable_new_recovery(durable_plan("run-source"));
    let proof = source
        .durability_proof()
        .expect("source proof must be complete");

    let destination = fully_durable_new_recovery(durable_plan("run-destination"));
    assert_eq!(
        destination.finish_recovered_new(proof),
        Err(TransactionError::DurabilityProofMismatch)
    );
}

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
