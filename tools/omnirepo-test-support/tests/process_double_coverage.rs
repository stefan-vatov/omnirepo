//! Focused edge coverage for the deterministic process double.
//!
//! These tests use barriers and process exit status as synchronization. They
//! do not use sleeps or wall-clock polling, so a failure points to a protocol
//! or cleanup contract rather than a timing race.

use std::path::Path;

use omnirepo_test_support::{
    lifecycle_fixture::{FixtureOutcome, FixtureSpec, LifecycleFixture},
    process_double::{FakeExecutable, ProcessBehavior, ProcessDoubleError, ProcessSpec},
};

fn process_capability_or_skip() -> bool {
    #[cfg(unix)]
    {
        if !Path::new("/bin/sh").is_file() {
            eprintln!("SKIP process coverage: /bin/sh is unavailable");
            return false;
        }
        true
    }
    #[cfg(not(unix))]
    {
        eprintln!("SKIP process coverage: Unix process semantics are unavailable");
        false
    }
}

fn assert_protocol(error: ProcessDoubleError, expected: &str) {
    assert_eq!(
        error.to_string(),
        format!("process double protocol error: {expected}")
    );
}

fn cleanup(fixture: LifecycleFixture) {
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed, "successful fixtures should be removed");
    assert!(
        report.leaks.is_empty(),
        "unexpected fixture residue: {:?}",
        report.leaks
    );
}

#[test]
fn lifecycle_protocol_rejects_wrong_order_and_repeated_reads() {
    if !process_capability_or_skip() {
        return;
    }

    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-ordering", 7310))
        .expect("fixture should be created");
    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("ordering", ProcessBehavior::Barrier),
    )
    .expect("process should spawn");

    assert_protocol(
        process
            .release()
            .expect_err("release before the barrier must fail"),
        "release called before barrier hit",
    );
    assert_protocol(
        process.wait().expect_err("wait before release must fail"),
        "wait called before deterministic release",
    );

    process
        .wait_for_barrier()
        .expect("barrier marker should be observed");
    process.release().expect("release should be accepted once");
    assert_protocol(
        process.release().expect_err("a second release must fail"),
        "release called more than once",
    );
    let result = process.wait().expect("released process should be reaped");
    assert!(result.status.success());
    assert_protocol(
        process
            .wait()
            .expect_err("a second wait must fail after reaping"),
        "child was already reaped",
    );
    assert_protocol(
        process
            .try_wait()
            .expect_err("try_wait after reaping must fail"),
        "child was already reaped",
    );

    cleanup(fixture);
}

#[test]
fn output_limits_apply_to_both_streams_after_multiple_chunks() {
    if !process_capability_or_skip() {
        return;
    }

    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-output-edges", 7311))
        .expect("fixture should be created");
    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new(
            "both-streams",
            ProcessBehavior::OversizedStdoutAndStderr {
                stdout_chunks: vec!["out".to_owned(), "put".to_owned(), "-tail".to_owned()],
                stderr_chunks: vec!["err".to_owned(), "or".to_owned(), "-tail".to_owned()],
            },
        )
        .with_output_limit(5),
    )
    .expect("process should spawn");
    process
        .wait_for_barrier()
        .expect("barrier marker should be observed");
    process.release().expect("release should be explicit");
    let result = process.wait().expect("process should be reaped");

    assert_eq!(result.stdout.bytes, b"outpu");
    assert!(result.stdout.truncated);
    assert_eq!(result.stderr.bytes, b"error");
    assert!(result.stderr.truncated);
    cleanup(fixture);
}

#[test]
fn shell_quote_roundtrips_apostrophes_without_rewriting_text() {
    if !process_capability_or_skip() {
        return;
    }

    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-quote", 7312))
        .expect("fixture should be created");
    let chunks = vec!["it's ready".to_owned(), " and it's exact".to_owned()];
    let expected = chunks.concat();
    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("apostrophe", ProcessBehavior::OversizedChunked { chunks }),
    )
    .expect("process should spawn");
    process
        .wait_for_barrier()
        .expect("barrier marker should be observed");
    process.release().expect("release should be explicit");
    let result = process.wait().expect("process should be reaped");

    assert_eq!(result.stdout.bytes, expected.as_bytes());
    assert!(!result.stdout.truncated);
    assert!(result.status.success());
    cleanup(fixture);
}

#[cfg(unix)]
#[test]
fn supported_unix_signals_map_to_their_exact_exit_statuses() {
    if !process_capability_or_skip() {
        return;
    }

    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-signals", 7313))
        .expect("fixture should be created");
    for (number, expected_signal) in [(1_u8, 1_i32), (2, 2), (9, 9), (13, 13), (15, 15)] {
        let mut process = FakeExecutable::spawn(
            &mut fixture,
            ProcessSpec::new(
                format!("signal-{number}"),
                ProcessBehavior::Signal { number },
            ),
        )
        .expect("signal process should spawn");
        process
            .wait_for_barrier()
            .expect("signal process should reach its barrier");
        process
            .release()
            .expect("signal release should be explicit");
        let result = process.wait().expect("signal process should be reaped");
        assert_eq!(result.status.code, None);
        assert_eq!(result.status.signal, Some(expected_signal));
        assert!(!result.status.success());
    }

    // Unsupported values use the fixture's documented safe TERM fallback.
    let mut fallback = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("signal-fallback", ProcessBehavior::Signal { number: 42 }),
    )
    .expect("fallback process should spawn");
    fallback
        .wait_for_barrier()
        .expect("fallback process should reach its barrier");
    fallback
        .release()
        .expect("fallback release should be explicit");
    assert_eq!(
        fallback
            .wait()
            .expect("fallback process should be reaped")
            .status
            .signal,
        Some(15)
    );

    cleanup(fixture);
}

#[test]
fn dropped_and_completed_process_trees_leave_no_fixture_residue() {
    if !process_capability_or_skip() {
        return;
    }

    let mut dropped_fixture = LifecycleFixture::create(FixtureSpec::new("process-drop", 7314))
        .expect("drop fixture should be created");
    let root = dropped_fixture.roots().root().to_path_buf();
    let mut hanging = FakeExecutable::spawn(
        &mut dropped_fixture,
        ProcessSpec::new("drop", ProcessBehavior::Hang),
    )
    .expect("hang process should spawn");
    hanging
        .wait_for_barrier()
        .expect("hang process should reach its barrier");
    drop(hanging);
    let report = dropped_fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    assert!(
        !root.exists(),
        "dropped process fixture root should be removed"
    );

    let mut fork_fixture = LifecycleFixture::create(FixtureSpec::new("process-fork-clean", 7315))
        .expect("fork fixture should be created");
    let mut fork = FakeExecutable::spawn(
        &mut fork_fixture,
        ProcessSpec::new("fork", ProcessBehavior::ForkAndLateWrite),
    )
    .expect("fork process should spawn");
    fork.wait_for_barrier()
        .expect("fork barrier should be observed");
    fork.release().expect("fork release should be explicit");
    let result = fork.wait().expect("fork process tree should be reaped");
    assert!(result.status.success());
    assert!(result.evidence.late_write);
    cleanup(fork_fixture);
}
