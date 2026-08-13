// Recovery-control tests are owned by the private support crate.

use std::{
    error::Error,
    fs,
    io::{self, ErrorKind},
};

use git_double::LocalGitRemoteDouble;
use lifecycle_fixture::{FixtureOutcome, FixtureSpec, LifecycleFixture};
use recovery_control::{
    CleanupRaceControl, ConcurrentRunControl, CrashDisposition, CrashSpec, CrashableParent,
    JournalControl, JournalTail, JournalWriterControl, LeaseControl, PidReuseControl,
    RecoveryError, RetainedState, WriterError, WriterFault,
};

use omnirepo_test_support::{git_double, lifecycle_fixture, recovery_control};

fn unix_or_skip() -> bool {
    if cfg!(unix) {
        true
    } else {
        eprintln!("SKIP recovery process controls: Unix executable semantics unavailable");
        false
    }
}

#[test]
fn crash_at_named_durable_boundary_is_replayed_after_restart() {
    if !unix_or_skip() {
        return;
    }
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new("crash-restart-tracer", 501).retain_always())
            .expect("fixture should be created");
    let spec = CrashSpec::at("journal.after_flush")
        .run_id("run-crash-restart")
        .with_state("stage", "journal-flushed")
        .with_state("terminal", "false")
        .disposition(CrashDisposition::Exit(137));
    let mut parent = CrashableParent::spawn(&mut fixture, spec).expect("parent should spawn");

    parent
        .wait_for_boundary()
        .expect("named boundary should be observable");
    let crash = parent.wait().expect("crashed parent should be reaped");
    assert_eq!(crash.boundary, "journal.after_flush");
    assert_eq!(crash.status.code, Some(137));

    let retained = RetainedState::restart(&fixture, "run-crash-restart")
        .expect("restart should read retained state");
    assert_eq!(retained.run_id, "run-crash-restart");
    assert_eq!(retained.boundary, "journal.after_flush");
    assert_eq!(retained.field("stage"), Some("journal-flushed"));
    assert_eq!(retained.field("terminal"), Some("false"));

    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    fs::remove_dir_all(report.root).expect("retained fixture should be removable");
}

#[test]
fn pid_reuse_and_stale_lease_observations_are_deterministic() {
    let mut pids = PidReuseControl::default();
    let original = pids.allocate(4_201);
    assert!(pids.release(&original));
    let (reused, evidence) = pids.reuse(4_201).expect("released PID should be reusable");
    assert_eq!(original.pid, reused.pid);
    assert_ne!(original.incarnation, reused.incarnation);
    assert_eq!(evidence.previous_incarnation, original.incarnation);
    assert_eq!(evidence.reused_incarnation, reused.incarnation);

    let mut leases = LeaseControl::default();
    let lease = leases
        .acquire("destination-a", original)
        .expect("lease should be acquired");
    leases
        .mark_unfinalized(&lease.lease_id)
        .expect("lease journal should become unfinalized");
    let observation = leases
        .inspect(&lease.lease_id, Some(&reused))
        .expect("stale lease should be observable");
    assert!(!observation.owner_matches);
    assert!(observation.journal_unfinalized);
    assert!(observation.stale_candidate);
}

#[test]
fn independent_concurrent_runs_reach_terminal_state_without_sleep() {
    if !unix_or_skip() {
        return;
    }
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("concurrent-runs", 502))
        .expect("fixture should be created");
    let mut runs = ConcurrentRunControl::launch(
        &mut fixture,
        ["run-a".to_owned(), "run-b".to_owned(), "run-c".to_owned()],
    )
    .expect("independent runs should launch");
    runs.wait_for_ready().expect("all run barriers should hit");
    runs.release_all().expect("all run barriers should release");
    let results = runs.join().expect("all run processes should be reaped");
    assert_eq!(results.len(), 3);
    for result in results {
        assert_eq!(result.status.code, Some(0));
        assert_eq!(result.state.get("run_id"), Some(&result.run_id));
        assert_eq!(result.state.get("status"), Some(&"completed".to_owned()));
    }
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn truncated_journal_tail_is_preserved_as_recovery_evidence() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("journal-tail", 503))
        .expect("fixture should be created");
    let journal = JournalControl::create(&mut fixture, "tail-run").expect("journal should create");
    journal
        .append_record("stage=started")
        .expect("first journal record should write");
    journal
        .append_record("stage=committing")
        .expect("second journal record should write");
    assert_eq!(journal.inspect().unwrap().tail, JournalTail::Complete);
    journal.truncate_tail(4).expect("tail should truncate");
    let evidence = journal.inspect().expect("truncated journal should read");
    assert_eq!(evidence.tail, JournalTail::Truncated);
    assert_eq!(evidence.records, vec!["stage=started", "stage=committ"]);
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn post_commit_crash_retains_commit_identity_for_restart() {
    if !unix_or_skip() {
        return;
    }
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new("post-commit-crash", 504).retain_always())
            .expect("fixture should be created");
    let spec = CrashSpec::at("git.after_commit")
        .run_id("post-commit-run")
        .with_state("commit_oid", "deadbeef")
        .with_state("remote", "not-observed")
        .disposition(CrashDisposition::Exit(137));
    let mut parent = CrashableParent::spawn(&mut fixture, spec).expect("parent should spawn");
    parent
        .wait_for_boundary()
        .expect("commit boundary should hit");
    let crash = parent.wait().expect("parent should be reaped");
    assert_eq!(crash.status.code, Some(137));
    let retained = RetainedState::restart(&fixture, "post-commit-run")
        .expect("restart should retain commit state");
    assert_eq!(retained.field("commit_oid"), Some("deadbeef"));
    assert_eq!(retained.field("remote"), Some("not-observed"));
    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    fs::remove_dir_all(report.root).expect("retained fixture should be removable");
}

#[test]
fn accepted_then_disconnected_remote_is_replayable_without_real_push() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("remote-disconnect", 505))
        .expect("fixture should be created");
    let remote = LocalGitRemoteDouble::bind(&mut fixture, "recovery-remote")
        .expect("local remote should bind");
    let attempt = remote
        .begin_attempt(b"update deadbeef refs/heads/main\0")
        .expect("local push attempt should start");
    let accepted = remote
        .wait_for_accept()
        .expect("remote acceptance should be observable");
    assert!(accepted.accepted);
    remote.disconnect().expect("disconnect should be explicit");
    let final_evidence = remote.finish().expect("remote should be reaped");
    assert!(final_evidence.disconnected);
    assert_eq!(final_evidence.payload, accepted.payload);
    assert!(attempt.join().expect("attempt should be reaped").is_empty());
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn writer_failures_keep_partial_evidence_and_do_not_fill_the_disk() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("writer-faults", 506))
        .expect("fixture should be created");
    let mut enospc =
        JournalWriterControl::create(&mut fixture, "enospc", Some(WriterFault::EnospcAfter(4)))
            .expect("ENOSPC writer should create");
    let error = enospc
        .write(b"abcdefgh")
        .expect_err("writer should inject ENOSPC");
    assert_eq!(
        error,
        WriterError::Enospc {
            attempted: 8,
            written: 4
        }
    );
    assert_eq!(fs::read(enospc.path()).unwrap(), b"abcd");

    let mut failed = JournalWriterControl::create(
        &mut fixture,
        "writer-error",
        Some(WriterFault::Error("journal closed".to_owned())),
    )
    .expect("writer error control should create");
    assert_eq!(
        failed.write(b"record").unwrap_err(),
        WriterError::Injected("journal closed".to_owned())
    );
    assert!(fs::read(failed.path()).unwrap().is_empty());
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn cleanup_race_orders_residue_before_late_writer() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("cleanup-race", 507))
        .expect("fixture should be created");
    let race =
        CleanupRaceControl::start(&mut fixture, "late-writer").expect("cleanup race should start");
    fs::write(race.path(), b"old-residue\n").expect("old residue should exist");
    race.wait_for_writer().expect("writer barrier should hit");
    let observation = race
        .cleanup_before_writer()
        .expect("cleanup observation should be captured");
    assert!(observation.existed_before_cleanup);
    assert!(observation.removed_before_writer);
    race.release_writer().expect("late writer should release");
    let evidence = race.join().expect("late writer should be joined");
    assert!(evidence.writer_started);
    assert!(evidence.writer_completed);
    assert_eq!(fs::read(evidence.path).unwrap(), b"late-writer\n");
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn recovery_error_projections_and_boundary_validation_are_stable() {
    let io_error = RecoveryError::from(io::Error::new(ErrorKind::NotFound, "missing journal"));
    assert_eq!(
        io_error.to_string(),
        "recovery control I/O error: missing journal"
    );
    assert!(io_error.source().is_none());

    let fixture_error = RecoveryError::from(lifecycle_fixture::FixtureError::Invariant(
        "broken fixture".to_owned(),
    ));
    assert_eq!(
        fixture_error.to_string(),
        "recovery control fixture error: fixture invariant failed: broken fixture"
    );
    assert_eq!(
        RecoveryError::Protocol("bad marker".to_owned()).to_string(),
        "recovery control protocol error: bad marker"
    );
    assert_eq!(
        RecoveryError::Thread("worker stopped".to_owned()).to_string(),
        "recovery control thread error: worker stopped"
    );
    assert_eq!(
        RecoveryError::Writer(WriterError::Injected("closed".to_owned())).to_string(),
        "recovery control writer error: simulated writer failure: closed"
    );

    if !unix_or_skip() {
        return;
    }
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("recovery-validation", 508))
        .expect("fixture should be created");

    let invalid_run = CrashableParent::spawn(&mut fixture, CrashSpec::at("boundary").run_id(""))
        .err()
        .expect("empty run IDs must be rejected");
    assert_eq!(
        invalid_run.to_string(),
        "recovery control protocol error: invalid run_id: \"\""
    );
    let invalid_boundary = CrashableParent::spawn(&mut fixture, CrashSpec::at("bad/boundary"))
        .err()
        .expect("path separators must be rejected in boundaries");
    assert_eq!(
        invalid_boundary.to_string(),
        "recovery control protocol error: invalid boundary: \"bad/boundary\""
    );
    let invalid_key = CrashableParent::spawn(
        &mut fixture,
        CrashSpec::at("boundary").with_state("state/key", "value"),
    )
    .err()
    .expect("path separators must be rejected in state keys");
    assert_eq!(
        invalid_key.to_string(),
        "recovery control protocol error: invalid state key: \"state/key\""
    );
    let invalid_value = CrashableParent::spawn(
        &mut fixture,
        CrashSpec::at("boundary").with_state("state", "line\nvalue"),
    )
    .err()
    .expect("newlines must be rejected in state values");
    assert_eq!(
        invalid_value.to_string(),
        "recovery control protocol error: invalid state value: newline is not allowed"
    );

    let invalid_concurrent = ConcurrentRunControl::launch(&mut fixture, ["bad/run".to_owned()])
        .err()
        .expect("path separators must be rejected in concurrent run IDs");
    assert!(invalid_concurrent.to_string().contains("invalid run_id"));
    let invalid_journal = JournalControl::create(&mut fixture, "../journal")
        .err()
        .expect("path traversal must be rejected in journal IDs");
    assert!(invalid_journal.to_string().contains("invalid run_id"));
    let invalid_writer = JournalWriterControl::create(&mut fixture, "", None)
        .err()
        .expect("empty writer names must be rejected");
    assert!(invalid_writer.to_string().contains("invalid writer name"));
    let invalid_cleanup = CleanupRaceControl::start(&mut fixture, "../late")
        .err()
        .expect("path traversal must be rejected in cleanup names");
    assert!(invalid_cleanup.to_string().contains("invalid cleanup name"));

    let mut leases = LeaseControl::default();
    let invalid_lease = leases
        .acquire(
            "destination/a",
            recovery_control::ProcessIdentity {
                pid: 1,
                incarnation: 1,
            },
        )
        .expect_err("path separators must be rejected in repositories");
    assert!(invalid_lease.to_string().contains("invalid repository"));
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn crash_signals_quoting_and_already_reaped_protocol_are_exact() {
    if !unix_or_skip() {
        return;
    }
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new("crash-signals", 509).retain_always())
            .expect("fixture should be created");

    for (index, signal) in [1_u8, 2, 9, 13, 15, 99].into_iter().enumerate() {
        let run_id = format!("signal-run-{index}");
        let mut parent = CrashableParent::spawn(
            &mut fixture,
            CrashSpec::at("signal boundary")
                .run_id(&run_id)
                .with_state("quoted", "O'Reilly $HOME; keep exact")
                .disposition(CrashDisposition::Signal(signal)),
        )
        .expect("signal parent should spawn");
        parent
            .wait_for_boundary()
            .expect("durable marker should be observed before signal");
        let crash = parent.wait().expect("signal parent should be reaped");
        let expected_signal = if signal == 99 { 15 } else { i32::from(signal) };
        assert_eq!(crash.status.signal, Some(expected_signal));
        let retained = RetainedState::restart(&fixture, &run_id)
            .expect("quoted retained state should be readable");
        assert_eq!(retained.boundary, "signal boundary");
        assert_eq!(retained.field("quoted"), Some("O'Reilly $HOME; keep exact"));
        assert_eq!(retained.field("missing"), None);
        assert!(parent.wait().is_err(), "waiting twice must fail closed");
    }

    let mut parent = CrashableParent::spawn(
        &mut fixture,
        CrashSpec::at("try-wait")
            .run_id("try-wait-run")
            .disposition(CrashDisposition::Exit(0)),
    )
    .expect("try-wait parent should spawn");
    parent
        .wait_for_boundary()
        .expect("try-wait marker should be observed");
    let _ = parent
        .try_wait()
        .expect("try_wait should be callable after marker");
    let evidence = parent.wait().expect("try-wait parent should be reaped");
    assert_eq!(evidence.status.code, Some(0));

    let parent = CrashableParent::spawn(
        &mut fixture,
        CrashSpec::at("drop-before-wait").run_id("drop-run"),
    )
    .expect("drop test parent should spawn");
    drop(parent);

    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    fs::remove_dir_all(report.root).expect("retained crash fixture should be removable");
}

#[test]
fn retained_state_rejects_malformed_journal_and_preserves_fields() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("retained-validation", 510))
        .expect("fixture should be created");

    for invalid in ["", ".", "..", "bad/name", "bad\\name", "bad\nname"] {
        let error = RetainedState::restart(&fixture, invalid)
            .expect_err("invalid run IDs must fail before filesystem access");
        assert!(error.to_string().contains("invalid run_id"));
    }

    let missing = RetainedState::restart(&fixture, "missing-run")
        .expect_err("missing journal should produce an I/O error");
    assert!(matches!(missing, RecoveryError::Io(_)));

    let path = fixture.roots().runs().join("malformed-run.journal");
    fs::write(&path, "not-a-record\n").expect("malformed journal should be writable");
    let malformed = RetainedState::restart(&fixture, "malformed-run")
        .expect_err("records without separators must fail");
    assert!(malformed.to_string().contains("no key/value separator"));

    fs::write(&path, "boundary=only\n").expect("missing run record should be writable");
    let missing_run =
        RetainedState::restart(&fixture, "malformed-run").expect_err("run_id is required");
    assert!(missing_run.to_string().contains("has no run_id"));

    fs::write(&path, "run_id=other\nboundary=boundary\n")
        .expect("mismatched retained journal should be writable");
    let mismatch = RetainedState::restart(&fixture, "malformed-run")
        .expect_err("run identity mismatch must fail closed");
    assert!(mismatch.to_string().contains("run_id mismatch"));

    fs::write(&path, "run_id=malformed-run\n").expect("missing boundary should be writable");
    let missing_boundary =
        RetainedState::restart(&fixture, "malformed-run").expect_err("boundary is required");
    assert!(missing_boundary.to_string().contains("has no boundary"));

    fs::write(
        &path,
        "run_id=malformed-run\nboundary=stable\nrun_id=duplicate\n",
    )
    .expect("duplicate retained keys should be writable");
    let duplicate = RetainedState::restart(&fixture, "malformed-run")
        .expect_err("duplicate keys must fail closed");
    assert!(
        duplicate
            .to_string()
            .contains("duplicate or empty record key")
    );

    fs::write(
        &path,
        "run_id=malformed-run\nboundary=stable\nextra=value\n",
    )
    .expect("valid retained journal should be writable");
    let retained = RetainedState::restart(&fixture, "malformed-run")
        .expect("valid retained state should load");
    assert_eq!(retained.path, path);
    assert_eq!(retained.boundary, "stable");
    assert_eq!(retained.field("extra"), Some("value"));
    assert_eq!(retained.field("version"), None);
    assert_eq!(retained.field("missing"), None);

    fs::write(&path, "=empty-key\n").expect("empty keys should be writable");
    let empty_key =
        RetainedState::restart(&fixture, "malformed-run").expect_err("empty keys must fail closed");
    assert!(
        empty_key
            .to_string()
            .contains("duplicate or empty record key")
    );
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn pid_and_lease_controls_reject_stale_and_duplicate_owners() {
    let mut pids = PidReuseControl::default();
    assert_eq!(pids.current(4_211), None);
    assert!(pids.evidence().is_empty());

    let original = pids.allocate(4_211);
    assert!(!pids.release(&recovery_control::ProcessIdentity {
        pid: original.pid,
        incarnation: original.incarnation + 1,
    }));
    let active = pids
        .reuse(original.pid)
        .expect_err("active process IDs cannot be reused");
    assert!(active.to_string().contains("still active"));
    assert!(pids.release(&original));
    assert!(!pids.release(&original));
    let (reused, evidence) = pids
        .reuse(original.pid)
        .expect("released PID can be reused");
    assert_eq!(pids.current(reused.pid), Some(&reused));
    assert_eq!(pids.evidence(), std::slice::from_ref(&evidence));

    let mut leases = LeaseControl::default();
    let lease = leases
        .acquire("destination-b", original.clone())
        .expect("lease should be acquired");
    let duplicate = leases
        .acquire("destination-b", original)
        .expect_err("duplicate lease IDs must fail closed");
    assert!(duplicate.to_string().contains("lease already exists"));

    let active_owner = leases
        .inspect(&lease.lease_id, Some(&lease.owner))
        .expect("active lease should be inspectable");
    assert!(active_owner.owner_matches);
    assert!(!active_owner.journal_unfinalized);
    assert!(!active_owner.stale_candidate);
    let missing_owner = leases
        .inspect(&lease.lease_id, None)
        .expect("lease can be inspected without an owner observation");
    assert!(!missing_owner.owner_matches);
    assert!(!missing_owner.journal_unfinalized);
    assert!(!missing_owner.stale_candidate);

    let unknown_mark = leases
        .mark_unfinalized("unknown-lease")
        .expect_err("unknown leases cannot be mutated");
    assert!(unknown_mark.to_string().contains("unknown lease"));
    let unknown_inspect = leases
        .inspect("unknown-lease", None)
        .expect_err("unknown leases cannot be inspected");
    assert!(unknown_inspect.to_string().contains("unknown lease"));
}

#[test]
fn concurrent_runs_reject_duplicate_ids_and_drop_reaps_children() {
    if !unix_or_skip() {
        return;
    }
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("concurrent-validation", 511))
        .expect("fixture should be created");
    let duplicate =
        ConcurrentRunControl::launch(&mut fixture, ["same-run".to_owned(), "same-run".to_owned()])
            .err()
            .expect("duplicate concurrent IDs must fail closed");
    assert!(duplicate.to_string().contains("duplicate concurrent run"));

    let mut empty = ConcurrentRunControl::launch(&mut fixture, std::iter::empty())
        .expect("empty run sets should be valid");
    empty
        .wait_for_ready()
        .expect("empty ready barrier should pass");
    empty.release_all().expect("empty release should pass");
    assert!(empty.join().expect("empty run set should join").is_empty());

    let runs = ConcurrentRunControl::launch(&mut fixture, ["drop-run".to_owned()])
        .expect("drop run should launch");
    drop(runs);
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn partial_concurrent_launch_reaps_prior_children_and_owned_state() {
    if !unix_or_skip() {
        return;
    }

    for (case, run_ids, expected) in [
        (
            "partial-invalid",
            vec!["first-run".to_owned(), "bad/run".to_owned()],
            "invalid run_id",
        ),
        (
            "partial-duplicate",
            vec!["first-run".to_owned(), "first-run".to_owned()],
            "duplicate concurrent run",
        ),
    ] {
        let mut fixture = LifecycleFixture::create(FixtureSpec::new(case, 515))
            .expect("fixture should be created");
        let error = ConcurrentRunControl::launch(&mut fixture, run_ids)
            .err()
            .expect("partial launch should fail at the later input");
        assert!(error.to_string().contains(expected));
        assert_eq!(
            fs::read_dir(fixture.roots().runs())
                .expect("fixture runs root should be readable")
                .count(),
            0,
            "failed partial launch must not leave concurrent state residue"
        );
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
    }
}

#[test]
fn journal_control_rejects_invalid_records_and_reports_io_failures() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("journal-validation", 512))
        .expect("fixture should be created");
    let journal =
        JournalControl::create(&mut fixture, "journal-validation").expect("journal should create");
    assert_eq!(
        journal
            .inspect()
            .expect("empty journal should inspect")
            .tail,
        JournalTail::Complete
    );

    for invalid in ["contains\nnewline", "contains\rreturn"] {
        let error = journal
            .append_record(invalid)
            .expect_err("journal records cannot contain line breaks");
        assert!(error.to_string().contains("invalid journal record"));
    }
    journal
        .append_record("record=value")
        .expect("valid journal record should append");
    let evidence = journal.inspect().expect("journal should inspect");
    assert_eq!(evidence.records, vec!["record=value"]);
    assert_eq!(evidence.bytes, b"record=value\n");
    journal
        .truncate_tail(10_000)
        .expect("oversized truncation should clamp to zero");
    assert_eq!(
        journal
            .inspect()
            .expect("truncated empty journal should inspect")
            .tail,
        JournalTail::Complete
    );

    let path = fixture.roots().runs().join("journal-validation.jsonl");
    fs::remove_file(&path).expect("journal path should exist");
    let inspect_error = journal
        .inspect()
        .expect_err("missing journal should produce an I/O error");
    assert!(matches!(inspect_error, RecoveryError::Io(_)));
    let append_error = journal
        .append_record("after-remove")
        .expect_err("appending to missing journal should fail");
    assert!(matches!(append_error, RecoveryError::Io(_)));
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn writer_success_and_faults_are_consumed_without_unbounded_output() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("writer-validation", 513))
        .expect("fixture should be created");
    let mut success = JournalWriterControl::create(&mut fixture, "success", None)
        .expect("success writer should create");
    let evidence = success
        .write(b"record")
        .expect("success writer should write");
    assert_eq!(evidence.attempted, 6);
    assert_eq!(evidence.written, 6);
    assert!(!evidence.failed);
    assert_eq!(fs::read(success.path()).unwrap(), b"record");
    let second = success.write(b"next").expect("writer fault is not sticky");
    assert_eq!(second.written, 4);

    let mut zero =
        JournalWriterControl::create(&mut fixture, "zero", Some(WriterFault::EnospcAfter(0)))
            .expect("zero-limit writer should create");
    assert_eq!(
        zero.write(b"bytes").unwrap_err(),
        WriterError::Enospc {
            attempted: 5,
            written: 0
        }
    );
    assert!(fs::read(zero.path()).unwrap().is_empty());
    assert_eq!(
        zero.write(b"ok").expect("fault should be consumed").written,
        2
    );

    let mut injected = JournalWriterControl::create(
        &mut fixture,
        "injected",
        Some(WriterFault::Error("closed".to_owned())),
    )
    .expect("injected writer should create");
    assert_eq!(
        injected.write(b"bytes").unwrap_err(),
        WriterError::Injected("closed".to_owned())
    );
    assert_eq!(
        injected
            .write(b"ok")
            .expect("injected fault should be consumed")
            .written,
        2
    );

    let mut missing = JournalWriterControl::create(&mut fixture, "missing", None)
        .expect("missing-path writer should create");
    fs::remove_file(missing.path()).expect("writer path should exist");
    fs::create_dir(missing.path()).expect("writer path should be replaced by a directory");
    let missing_error = missing
        .write(b"bytes")
        .expect_err("writing to a directory path should fail");
    assert!(
        matches!(missing_error, WriterError::Injected(message) if message.starts_with("write failed:"))
    );
    fs::remove_dir_all(missing.path()).expect("writer directory residue should be removable");
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn cleanup_control_handles_no_residue_directory_failure_and_abort() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("cleanup-validation", 514))
        .expect("fixture should be created");
    let race =
        CleanupRaceControl::start(&mut fixture, "no-residue").expect("cleanup race should start");
    race.wait_for_writer().expect("writer barrier should hit");
    let observation = race
        .cleanup_before_writer()
        .expect("cleanup observation should be captured");
    assert!(!observation.existed_before_cleanup);
    assert!(!observation.removed_before_writer);
    race.release_writer().expect("writer should release");
    let evidence = race.join().expect("writer should join");
    assert!(evidence.writer_completed);
    assert_eq!(fs::read(evidence.path).unwrap(), b"late-writer\n");

    let directory =
        CleanupRaceControl::start(&mut fixture, "directory").expect("directory race should start");
    fs::create_dir(directory.path()).expect("residue directory should be created");
    directory
        .wait_for_writer()
        .expect("directory writer barrier should hit");
    let observation = directory
        .cleanup_before_writer()
        .expect("directory cleanup should be observed");
    assert!(observation.existed_before_cleanup);
    assert!(!observation.removed_before_writer);
    directory
        .release_writer()
        .expect("directory writer should release");
    let join_error = directory
        .join()
        .expect_err("writer cannot replace a residue directory");
    assert!(matches!(join_error, RecoveryError::Io(_)));
    fs::remove_dir_all(fixture.roots().artifacts().join("directory.late"))
        .expect("residue directory should be removable");

    let abort = CleanupRaceControl::start(&mut fixture, "abort").expect("abort race should start");
    drop(abort);
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}
