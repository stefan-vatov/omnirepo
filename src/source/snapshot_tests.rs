//! Deterministic, filesystem-free coverage for source snapshot lifecycle state.

use super::snapshot::*;
/*
    CacheKey, CleanupOutcome, FailureKind, Freshness, IdentityError, MaterializationLease,
    OperationId, PublishedSnapshot, RevisionId, SnapshotId, SnapshotState, SnapshotStore, SourceId,
    SourceIdentity, StagingId, TransitionError,
*/

fn source() -> SourceIdentity {
    SourceIdentity::new(
        SourceId::new("shared-config").expect("fixture source id"),
        "https://example.test/omnirepo-config.git",
    )
    .expect("fixture source identity")
}

fn revision_id(value: &str) -> RevisionId {
    RevisionId::new(value).expect("fixture revision")
}

fn published(revision: &str, snapshot: &str, cache: &str) -> PublishedSnapshot {
    PublishedSnapshot::new(
        source(),
        revision_id(revision),
        SnapshotId::new(snapshot).expect("fixture snapshot id"),
        CacheKey::new(cache).expect("fixture cache key"),
    )
}

fn lease(
    store: &mut SnapshotStore,
    operation: &str,
    staging: &str,
    cache: &str,
) -> MaterializationLease {
    store
        .begin(
            OperationId::new(operation).expect("fixture operation"),
            StagingId::new(staging).expect("fixture staging"),
            CacheKey::new(cache).expect("fixture cache"),
        )
        .expect("materializer lease")
}

#[test]
fn absent_snapshot_can_be_materialized_and_published_atomically() {
    let mut store = SnapshotStore::new(source());
    assert_eq!(store.state(), &SnapshotState::Absent);
    assert_eq!(store.reader(), None);

    let lease = lease(&mut store, "run-1", "stage-1", "cache-1");
    assert!(matches!(store.state(), SnapshotState::InProgress { .. }));
    assert_eq!(store.reader(), None);

    let next = published("rev-1", "snap-1", "cache-1");
    store
        .publish(&lease, next.clone())
        .expect("atomic publication");
    assert_eq!(store.state(), &SnapshotState::Complete(next.clone()));
    assert_eq!(store.published(), Some(&next));
    assert_eq!(store.reader().expect("published reader").snapshot(), &next);
}

#[test]
fn published_snapshot_can_be_classified_fresh_or_stale_without_mutating_bytes() {
    let snapshot = published("rev-1", "snap-1", "cache-1");
    let mut store = SnapshotStore::with_published(source(), snapshot.clone(), Freshness::Fresh)
        .expect("fresh store");
    assert_eq!(store.state(), &SnapshotState::Fresh(snapshot.clone()));
    assert_eq!(store.reader().expect("fresh reader").snapshot(), &snapshot);

    store
        .classify(Freshness::Stale)
        .expect("stale classification");
    assert_eq!(store.state(), &SnapshotState::Stale(snapshot.clone()));
    assert_eq!(store.published(), Some(&snapshot));
    store
        .classify(Freshness::Fresh)
        .expect("fresh reclassification");
    assert_eq!(store.state(), &SnapshotState::Fresh(snapshot));

    let mut complete = SnapshotStore::new(source());
    let complete_lease = lease(
        &mut complete,
        "complete",
        "stage-complete",
        "cache-complete",
    );
    let complete_snapshot = published("rev-complete", "snap-complete", "cache-complete");
    complete
        .publish(&complete_lease, complete_snapshot.clone())
        .expect("complete publication");
    complete
        .classify(Freshness::Stale)
        .expect("classify complete publication");
    assert_eq!(complete.state(), &SnapshotState::Stale(complete_snapshot));
}

#[test]
fn concurrent_materializers_are_rejected_but_diagnostics_keep_previous_snapshot() {
    let old = published("rev-old", "snap-old", "cache-old");
    let mut store = SnapshotStore::with_published(source(), old.clone(), Freshness::Fresh)
        .expect("old snapshot");
    let reader = store.reader().expect("reader before refresh");
    let first = lease(&mut store, "run-1", "stage-1", "cache-new");

    assert_eq!(store.reader(), None, "previous publication is not current");
    assert_eq!(store.diagnostic_snapshot(), Some(&old));
    let concurrent = store.begin(
        OperationId::new("run-2").expect("operation"),
        StagingId::new("stage-2").expect("staging"),
        CacheKey::new("cache-new-2").expect("cache"),
    );
    assert_eq!(concurrent, Err(TransitionError::ConcurrentMaterializer));

    let next = published("rev-new", "snap-new", "cache-new");
    store.publish(&first, next.clone()).expect("publish");
    assert_eq!(reader.snapshot(), &old);
    assert_eq!(store.reader().expect("new reader").snapshot(), &next);
}

#[test]
fn stale_lease_cannot_publish_after_interruption_and_recovery() {
    let mut store = SnapshotStore::new(source());
    let old_lease = lease(&mut store, "run-old", "stage-old", "cache-old");
    let cleanup = store
        .interrupt(
            &old_lease,
            "process interrupted",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("interrupt");
    assert_eq!(cleanup, CleanupOutcome::RetainedForRecovery);
    assert!(matches!(store.state(), SnapshotState::Interrupted { .. }));

    let new_lease = store
        .recover(
            OperationId::new("run-new").expect("operation"),
            StagingId::new("stage-new").expect("staging"),
            CacheKey::new("cache-new").expect("cache"),
        )
        .expect("explicit recovery");
    let error = store
        .publish(&old_lease, published("rev-old", "snap-old", "cache-old"))
        .expect_err("old lease must be rejected");
    assert_eq!(error, TransitionError::LeaseMismatch);
    store
        .publish(&new_lease, published("rev-new", "snap-new", "cache-new"))
        .expect("new lease publishes");
}

#[test]
fn failure_retains_previous_snapshot_and_records_cleanup_evidence() {
    let old = published("rev-old", "snap-old", "cache-old");
    let mut store = SnapshotStore::with_published(source(), old.clone(), Freshness::Fresh)
        .expect("old snapshot");
    let lease = lease(&mut store, "run-1", "stage-1", "cache-new");
    let cleanup = store
        .fail(
            &lease,
            FailureKind::FetchFailed,
            "fetch failed",
            CleanupOutcome::Removed,
        )
        .expect("failure transition");
    assert_eq!(cleanup, CleanupOutcome::Removed);
    assert!(matches!(store.state(), SnapshotState::Failed { .. }));
    assert_eq!(store.published(), Some(&old));
    assert_eq!(
        store.reader(),
        None,
        "failed state has no current authority"
    );
    assert_eq!(store.diagnostic_snapshot(), Some(&old));
}

#[test]
fn invalid_transitions_and_source_revision_mismatches_are_precise() {
    let mut store = SnapshotStore::new(source());
    let publish_without_lease = store.publish(
        &MaterializationLease::test_fixture("not-active"),
        published("rev-1", "snap-1", "cache-1"),
    );
    assert_eq!(publish_without_lease, Err(TransitionError::NotInProgress));

    let lease = lease(&mut store, "run-1", "stage-1", "cache-1");
    let other_source = SourceIdentity::new(
        SourceId::new("other").expect("source id"),
        "https://example.test/other.git",
    )
    .expect("source");
    let wrong = PublishedSnapshot::new(
        other_source,
        revision_id("rev-1"),
        SnapshotId::new("snap-1").expect("snapshot"),
        CacheKey::new("cache-1").expect("cache"),
    );
    assert_eq!(
        store.publish(&lease, wrong),
        Err(TransitionError::SourceIdentityMismatch)
    );
}

#[test]
fn cache_identity_is_owned_by_the_lease_and_source() {
    let snapshot = published("rev-1", "snap-1", "cache-1");
    let other_source = SourceIdentity::new(
        SourceId::new("other").expect("source id"),
        "https://example.test/other.git",
    )
    .expect("source");
    assert_eq!(
        SnapshotStore::with_published(other_source, snapshot, Freshness::Fresh),
        Err(TransitionError::SourceIdentityMismatch)
    );

    let mut store = SnapshotStore::new(source());
    let lease = lease(&mut store, "run-1", "stage-1", "cache-1");
    let wrong_cache = published("rev-1", "snap-1", "cache-other");
    assert_eq!(
        store.publish(&lease, wrong_cache),
        Err(TransitionError::CacheMismatch)
    );
    assert!(matches!(store.state(), SnapshotState::InProgress { .. }));
}

#[test]
fn interruption_requires_recovery_retention_and_invalid_classification_is_rejected() {
    let mut store = SnapshotStore::new(source());
    let lease = lease(&mut store, "run-1", "stage-1", "cache-1");
    assert_eq!(
        store.interrupt(&lease, "signal", CleanupOutcome::Removed),
        Err(TransitionError::InterruptedStagingMustBeRetained)
    );
    assert_eq!(
        store.classify(Freshness::Fresh),
        Err(TransitionError::NoPublishedSnapshot)
    );
    store
        .interrupt(&lease, "signal", CleanupOutcome::RetainedForRecovery)
        .expect("retained interruption");
    assert_eq!(
        store.classify(Freshness::Fresh),
        Err(TransitionError::NoPublishedSnapshot)
    );
}

#[test]
fn stale_snapshots_are_diagnostic_only_and_never_authority_readers() {
    let snapshot = published("rev-stale", "snap-stale", "cache-stale");
    let mut store = SnapshotStore::with_published(source(), snapshot.clone(), Freshness::Stale)
        .expect("stale store");
    assert_eq!(store.published(), Some(&snapshot));
    assert_eq!(store.reader(), None, "stale authority must not be readable");

    let lease = lease(&mut store, "run-1", "stage-1", "cache-new");
    assert_eq!(store.published(), Some(&snapshot));
    assert_eq!(
        store.reader(),
        None,
        "refresh must not expose stale authority"
    );
    store
        .fail(
            &lease,
            FailureKind::FetchFailed,
            "offline",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("failure");
    assert_eq!(
        store.reader(),
        None,
        "failed stale recovery stays unreadable"
    );
}

#[test]
fn authority_reader_exposes_only_current_fresh_publication_and_diagnostics_are_explicit() {
    let old = published("rev-old", "snap-old", "cache-old");
    let mut store = SnapshotStore::with_published(source(), old.clone(), Freshness::Fresh)
        .expect("fresh store");
    assert_eq!(store.reader().expect("current reader").snapshot(), &old);
    assert_eq!(store.diagnostic_snapshot(), Some(&old));

    let lease = lease(&mut store, "run-1", "stage-1", "cache-new");
    assert_eq!(store.reader(), None, "previous publication is not current");
    assert_eq!(store.diagnostic_snapshot(), Some(&old));
    let next = published("rev-new", "snap-new", "cache-new");
    store.publish(&lease, next.clone()).expect("publish");
    assert_eq!(
        store.reader().expect("new current reader").snapshot(),
        &next
    );
    assert_eq!(store.diagnostic_snapshot(), Some(&next));

    store
        .classify(Freshness::Stale)
        .expect("stale classification");
    assert_eq!(store.reader(), None, "stale current is not authority");
    assert_eq!(store.diagnostic_snapshot(), Some(&next));
}

#[test]
fn failed_and_interrupted_recovery_is_explicit_and_retains_all_evidence() {
    let mut store = SnapshotStore::new(source());
    let failed_lease = lease(&mut store, "run-failed", "stage-failed", "cache-failed");
    store
        .fail(
            &failed_lease,
            FailureKind::CacheCorrupt,
            "cache checksum mismatch",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("failed attempt");
    assert_eq!(
        store.begin(
            OperationId::new("implicit").expect("operation"),
            StagingId::new("stage-implicit").expect("staging"),
            CacheKey::new("cache-implicit").expect("cache"),
        ),
        Err(TransitionError::RecoveryRequired)
    );
    let recovered = store
        .recover(
            OperationId::new("run-recovered").expect("operation"),
            StagingId::new("stage-recovered").expect("staging"),
            CacheKey::new("cache-recovered").expect("cache"),
        )
        .expect("explicit failed recovery");
    store
        .interrupt(
            &recovered,
            "received cancellation",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("interrupted recovery");

    let evidence = store.recovery_evidence();
    assert_eq!(
        evidence.len(),
        2,
        "failed evidence must survive interruption"
    );
    assert!(matches!(
        &evidence[0],
        RecoveryEvidence::Failed {
            operation,
            kind: FailureKind::CacheCorrupt,
            message,
            cleanup: CleanupOutcome::RetainedForRecovery,
        } if operation.as_str() == "run-failed" && message == "cache checksum mismatch"
    ));
    assert!(matches!(
        &evidence[1],
        RecoveryEvidence::Interrupted {
            operation,
            staging,
            reason,
            cleanup: CleanupOutcome::RetainedForRecovery,
        } if operation.as_str() == "run-recovered"
            && staging.as_str() == "stage-recovered"
            && reason == "received cancellation"
    ));

    let final_lease = store
        .recover(
            OperationId::new("run-final").expect("operation"),
            StagingId::new("stage-final").expect("staging"),
            CacheKey::new("cache-final").expect("cache"),
        )
        .expect("explicit interrupted recovery");
    store
        .publish(
            &final_lease,
            published("rev-final", "snap-final", "cache-final"),
        )
        .expect("recovered publication");
    assert_eq!(store.recovery_evidence().len(), 2);
}

#[test]
fn replay_labels_are_deterministic_for_every_terminal_state() {
    let mut store = SnapshotStore::new(source());
    assert_eq!(store.replay_label(), "state=absent");
    let lease = lease(&mut store, "run-1", "stage-1", "cache-1");
    assert_eq!(
        store.replay_label(),
        "state=in-progress;operation=run-1;staging=stage-1"
    );
    store
        .fail(
            &lease,
            FailureKind::CacheCorrupt,
            "cache invalid",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("failure");
    assert_eq!(
        store.replay_label(),
        "state=failed;kind=cache-corrupt;cleanup=retained-for-recovery"
    );
}

#[test]
fn identity_boundaries_reject_empty_nul_and_invalid_source_values() {
    assert_eq!(
        SourceId::new(""),
        Err(IdentityError::Empty { field: "source id" })
    );
    assert_eq!(
        SourceId::new("shared/config"),
        Err(IdentityError::InvalidSourceId {
            value: "shared/config".to_owned()
        })
    );
    assert_eq!(
        SourceId::new("UpperSource"),
        Err(IdentityError::InvalidSourceId {
            value: "UpperSource".to_owned()
        })
    );
    assert_eq!(
        RevisionId::new(""),
        Err(IdentityError::Empty { field: "revision" })
    );
    assert_eq!(
        SnapshotId::new(""),
        Err(IdentityError::Empty {
            field: "snapshot id"
        })
    );
    assert_eq!(
        OperationId::new(""),
        Err(IdentityError::Empty {
            field: "operation id"
        })
    );
    assert_eq!(
        StagingId::new(""),
        Err(IdentityError::Empty {
            field: "staging id"
        })
    );
    assert_eq!(
        CacheKey::new(""),
        Err(IdentityError::Empty { field: "cache key" })
    );

    assert_eq!(
        RevisionId::new("rev\0ision"),
        Err(IdentityError::ContainsNul { field: "revision" })
    );
    assert_eq!(
        SnapshotId::new("snap\0shot"),
        Err(IdentityError::ContainsNul {
            field: "snapshot id"
        })
    );
    assert_eq!(
        OperationId::new("op\0eration"),
        Err(IdentityError::ContainsNul {
            field: "operation id"
        })
    );
    assert_eq!(
        StagingId::new("stage\0ing"),
        Err(IdentityError::ContainsNul {
            field: "staging id"
        })
    );
    assert_eq!(
        CacheKey::new("cache\0key"),
        Err(IdentityError::ContainsNul { field: "cache key" })
    );

    let source_id = SourceId::new("valid-source").expect("source id");
    assert_eq!(
        SourceIdentity::new(source_id.clone(), ""),
        Err(IdentityError::Empty {
            field: "source locator"
        })
    );
    assert_eq!(
        SourceIdentity::new(source_id, "remote\0locator"),
        Err(IdentityError::ContainsNul {
            field: "source locator"
        })
    );
}

#[test]
fn recovery_is_explicit_and_rejected_before_a_failure_or_interruption() {
    let mut absent = SnapshotStore::new(source());
    assert_eq!(
        absent.recover(
            OperationId::new("recover-absent").expect("operation"),
            StagingId::new("stage-absent").expect("staging"),
            CacheKey::new("cache-absent").expect("cache"),
        ),
        Err(TransitionError::NoPublishedSnapshot)
    );

    for freshness in [Freshness::Fresh, Freshness::Stale] {
        let snapshot = published("rev-existing", "snap-existing", "cache-existing");
        let mut store =
            SnapshotStore::with_published(source(), snapshot, freshness).expect("published state");
        assert_eq!(
            store.recover(
                OperationId::new("recover-existing").expect("operation"),
                StagingId::new("stage-existing").expect("staging"),
                CacheKey::new("cache-existing").expect("cache"),
            ),
            Err(TransitionError::NoPublishedSnapshot)
        );
    }

    let mut in_progress = SnapshotStore::new(source());
    let active = lease(&mut in_progress, "active", "stage-active", "cache-active");
    assert_eq!(
        in_progress.recover(
            OperationId::new("recover-active").expect("operation"),
            StagingId::new("stage-recover-active").expect("staging"),
            CacheKey::new("cache-recover-active").expect("cache"),
        ),
        Err(TransitionError::NoPublishedSnapshot)
    );
    assert!(matches!(
        in_progress.state(),
        SnapshotState::InProgress { lease, .. } if lease == &active
    ));

    let mut complete = SnapshotStore::new(source());
    let first = lease(&mut complete, "first", "stage-first", "cache-first");
    complete
        .publish(&first, published("rev-first", "snap-first", "cache-first"))
        .expect("complete publication");
    assert_eq!(
        complete.recover(
            OperationId::new("recover-complete").expect("operation"),
            StagingId::new("stage-recover-complete").expect("staging"),
            CacheKey::new("cache-recover-complete").expect("cache"),
        ),
        Err(TransitionError::NoPublishedSnapshot)
    );
}

#[test]
fn wrong_leases_and_terminal_operations_leave_state_unchanged() {
    let mut store = SnapshotStore::new(source());
    let active = lease(&mut store, "active", "stage-active", "cache-active");
    let wrong = MaterializationLease::test_fixture("wrong");

    assert_eq!(
        store.fail(
            &wrong,
            FailureKind::PublicationFailed,
            "wrong lease",
            CleanupOutcome::NothingToRemove,
        ),
        Err(TransitionError::LeaseMismatch)
    );
    assert_eq!(
        store.interrupt(&wrong, "wrong lease", CleanupOutcome::RetainedForRecovery,),
        Err(TransitionError::LeaseMismatch)
    );
    assert!(matches!(
        store.state(),
        SnapshotState::InProgress { lease, .. } if lease == &active
    ));

    store
        .fail(
            &active,
            FailureKind::CleanupFailed,
            "cleanup failed",
            CleanupOutcome::Failed {
                reason: "permission denied".to_owned(),
            },
        )
        .expect("active lease fails");
    assert_eq!(
        store.fail(
            &active,
            FailureKind::FetchFailed,
            "already terminal",
            CleanupOutcome::Removed,
        ),
        Err(TransitionError::NotInProgress)
    );
    assert_eq!(
        store.interrupt(
            &active,
            "already terminal",
            CleanupOutcome::RetainedForRecovery,
        ),
        Err(TransitionError::NotInProgress)
    );
}

#[test]
fn previous_publication_and_freshness_survive_stale_refresh_recovery() {
    let old = published("rev-old", "snap-old", "cache-old");
    let mut stale = SnapshotStore::with_published(source(), old.clone(), Freshness::Stale)
        .expect("stale publication");
    let stale_lease = lease(&mut stale, "stale-refresh", "stage-stale", "cache-new");
    match stale.state() {
        SnapshotState::InProgress {
            previous,
            previous_freshness,
            ..
        } => {
            assert_eq!(previous.as_ref(), Some(&old));
            assert_eq!(*previous_freshness, Some(Freshness::Stale));
        }
        state => panic!("expected in-progress state, got {state:?}"),
    }
    stale
        .fail(
            &stale_lease,
            FailureKind::FetchFailed,
            "offline",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("stale refresh failure");
    match stale.state() {
        SnapshotState::Failed {
            previous,
            previous_freshness,
            ..
        } => {
            assert_eq!(previous.as_ref(), Some(&old));
            assert_eq!(*previous_freshness, Some(Freshness::Stale));
        }
        state => panic!("expected failed state, got {state:?}"),
    }
    let recovered = stale
        .recover(
            OperationId::new("stale-recovery").expect("operation"),
            StagingId::new("stage-recovery").expect("staging"),
            CacheKey::new("cache-recovery").expect("cache"),
        )
        .expect("explicit recovery");
    match stale.state() {
        SnapshotState::InProgress {
            previous,
            previous_freshness,
            ..
        } => {
            assert_eq!(previous.as_ref(), Some(&old));
            assert_eq!(*previous_freshness, Some(Freshness::Stale));
        }
        state => panic!("expected recovered state, got {state:?}"),
    }
    stale
        .publish(
            &recovered,
            published("rev-new", "snap-new", "cache-recovery"),
        )
        .expect("recovered publication");
    assert_eq!(
        stale.published().expect("publication").revision().as_str(),
        "rev-new"
    );

    let mut complete = SnapshotStore::new(source());
    let first = lease(
        &mut complete,
        "complete-first",
        "stage-first",
        "cache-first",
    );
    let first_snapshot = published("rev-first", "snap-first", "cache-first");
    complete
        .publish(&first, first_snapshot.clone())
        .expect("first publication");
    let second = complete
        .begin(
            OperationId::new("complete-second").expect("operation"),
            StagingId::new("stage-second").expect("staging"),
            CacheKey::new("cache-second").expect("cache"),
        )
        .expect("refresh after complete");
    match complete.state() {
        SnapshotState::InProgress {
            previous,
            previous_freshness,
            ..
        } => {
            assert_eq!(previous.as_ref(), Some(&first_snapshot));
            assert_eq!(*previous_freshness, Some(Freshness::Fresh));
        }
        state => panic!("expected second in-progress state, got {state:?}"),
    }
    complete
        .interrupt(
            &second,
            "cancelled before publication",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("interrupt complete refresh");
}

#[test]
fn replay_labels_cover_fresh_stale_complete_and_interrupted_states() {
    let snapshot = published("rev-label", "snap-label", "cache-label");
    let mut fresh = SnapshotStore::with_published(source(), snapshot.clone(), Freshness::Fresh)
        .expect("fresh publication");
    assert_eq!(fresh.replay_label(), "state=fresh;revision=rev-label");
    fresh
        .classify(Freshness::Stale)
        .expect("stale classification");
    assert_eq!(fresh.replay_label(), "state=stale;revision=rev-label");

    let mut complete = SnapshotStore::new(source());
    let complete_lease = lease(&mut complete, "complete", "stage-complete", "cache-label");
    complete
        .publish(&complete_lease, snapshot)
        .expect("complete publication");
    assert_eq!(complete.replay_label(), "state=complete;revision=rev-label");

    let mut interrupted = SnapshotStore::new(source());
    let interrupted_lease = lease(
        &mut interrupted,
        "interrupted",
        "stage-interrupted",
        "cache-interrupted",
    );
    interrupted
        .interrupt(
            &interrupted_lease,
            "process interrupted",
            CleanupOutcome::RetainedForRecovery,
        )
        .expect("interruption");
    assert_eq!(
        interrupted.replay_label(),
        "state=interrupted;operation=interrupted;staging=stage-interrupted;cleanup=retained-for-recovery"
    );
}

#[test]
fn typed_failures_preserve_kind_cleanup_and_causal_evidence() {
    let cases = [
        (FailureKind::FetchFailed, "fetch-failed"),
        (FailureKind::CacheCorrupt, "cache-corrupt"),
        (FailureKind::PublicationFailed, "publication-failed"),
        (FailureKind::CleanupFailed, "cleanup-failed"),
    ];
    for (kind, label) in cases {
        let mut store = SnapshotStore::new(source());
        let lease = lease(&mut store, "typed-failure", "stage-typed", "cache-typed");
        let cleanup = CleanupOutcome::Failed {
            reason: "permission denied".to_owned(),
        };
        assert_eq!(
            store.fail(&lease, kind, "typed source failure", cleanup.clone()),
            Ok(cleanup.clone())
        );
        assert_eq!(
            store.replay_label(),
            format!("state=failed;kind={label};cleanup=failed")
        );
        assert!(matches!(
            &store.recovery_evidence()[0],
            RecoveryEvidence::Failed {
                operation,
                kind: recorded_kind,
                message,
                cleanup: recorded_cleanup,
            } if operation.as_str() == "typed-failure"
                && *recorded_kind == kind
                && message == "typed source failure"
                && recorded_cleanup == &cleanup
        ));
    }
}

#[test]
fn replay_diagnostics_do_not_expose_opaque_locator_credentials_or_paths() {
    let id = SourceId::new("remote-source").expect("source id");
    let locator = "ssh://user:secret@example.test/../outside.git";
    let identity = SourceIdentity::new(id, locator).expect("opaque locator");
    assert_eq!(identity.locator(), locator);

    let snapshot = PublishedSnapshot::new(
        identity.clone(),
        RevisionId::new("rev-safe").expect("revision"),
        SnapshotId::new("snap-safe").expect("snapshot"),
        CacheKey::new("cache-safe").expect("cache"),
    );
    let store = SnapshotStore::with_published(identity, snapshot, Freshness::Fresh)
        .expect("fresh publication");
    let replay = store.replay_label();
    assert_eq!(replay, "state=fresh;revision=rev-safe");
    assert!(!replay.contains("secret"));
    assert!(!replay.contains("outside"));
}
