// Deterministic Git-double coverage for the private test-support crate.

use std::{fs, io};

use git_double::{GitDoubleError, LocalGitRemoteDouble};
use lifecycle_fixture::{FixtureError, FixtureOutcome, FixtureSpec, LifecycleFixture};
use omnirepo_test_support::{git_double, lifecycle_fixture};

#[test]
fn git_double_errors_have_stable_display_and_conversion_paths() {
    let io_error =
        GitDoubleError::from(io::Error::new(io::ErrorKind::NotFound, "missing")).to_string();
    assert_eq!(io_error, "Git double I/O error: missing");

    let fixture_error = GitDoubleError::from(FixtureError::InvalidPath {
        path: "../outside".to_owned(),
        reason: "parent traversal is not accepted",
    })
    .to_string();
    assert_eq!(
        fixture_error,
        "Git double fixture error: invalid fixture path \"../outside\": parent traversal is not accepted"
    );

    for error in [
        GitDoubleError::Protocol("protocol failed".to_owned()),
        GitDoubleError::Thread("thread failed".to_owned()),
    ] {
        let rendered = error.to_string();
        assert!(rendered.starts_with("Git double "));
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn accepted_payload_is_replayed_and_evidence_is_written_locally() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("git-coverage-evidence", 8_101))
        .expect("fixture should be created");
    let remote =
        LocalGitRemoteDouble::bind(&mut fixture, "evidence").expect("local remote should bind");
    assert!(remote.endpoint().starts_with("git://127.0.0.1:"));

    let payload = b"update deadbeef refs/heads/main\0 report-status\n".to_vec();
    let attempt = remote
        .begin_attempt(payload.clone())
        .expect("push attempt should start");
    let accepted = remote
        .wait_for_accept()
        .expect("remote should expose accepted payload");
    assert_eq!(accepted.payload, payload);
    assert!(accepted.accepted);
    assert!(!accepted.disconnected);

    remote.disconnect().expect("disconnect should be explicit");
    let final_evidence = remote.finish().expect("remote should finish");
    assert_eq!(final_evidence.payload, payload);
    assert!(final_evidence.accepted);
    assert!(final_evidence.disconnected);
    assert!(
        attempt
            .join()
            .expect("push attempt should finish")
            .is_empty()
    );

    let evidence_path = fixture.roots().artifacts().join("evidence.git.evidence");
    assert_eq!(
        fs::read_to_string(evidence_path).expect("evidence should be persisted"),
        "accepted=true\ndisconnected=true\npayload_len=47\n"
    );
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn dropping_a_push_attempt_after_disconnect_reaps_the_client() {
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new("git-coverage-attempt-drop", 8_102))
            .expect("fixture should be created");
    let remote =
        LocalGitRemoteDouble::bind(&mut fixture, "attempt-drop").expect("local remote should bind");
    let attempt = remote
        .begin_attempt(b"drop-attempt")
        .expect("push attempt should start");
    remote
        .wait_for_accept()
        .expect("remote should accept the payload");

    remote
        .disconnect()
        .expect("disconnect should release the server");
    drop(attempt);
    let evidence = remote.finish().expect("remote should finish");
    assert!(evidence.disconnected);

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn dropping_a_remote_releases_the_server_and_reaps_an_active_attempt() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("git-coverage-remote-drop", 8_103))
        .expect("fixture should be created");
    let remote =
        LocalGitRemoteDouble::bind(&mut fixture, "remote-drop").expect("local remote should bind");
    let attempt = remote
        .begin_attempt(b"drop-remote")
        .expect("push attempt should start");
    remote
        .wait_for_accept()
        .expect("remote should accept the payload");
    let evidence_path = fixture.roots().artifacts().join("remote-drop.git.evidence");

    drop(remote);
    assert!(
        attempt
            .join()
            .expect("drop should close the client")
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(evidence_path).expect("drop should write evidence"),
        "accepted=true\ndisconnected=true\npayload_len=11\n"
    );

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn remote_join_preserves_evidence_write_failures_as_typed_errors() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("git-coverage-write-error", 8_104))
        .expect("fixture should be created");
    let remote =
        LocalGitRemoteDouble::bind(&mut fixture, "write-error").expect("local remote should bind");
    let attempt = remote
        .begin_attempt(b"write-error")
        .expect("push attempt should start");
    remote
        .wait_for_accept()
        .expect("remote should accept the payload");
    fs::remove_dir_all(fixture.roots().artifacts()).expect("artifact root should be removable");

    remote
        .disconnect()
        .expect("disconnect should release the server");
    let error = remote
        .finish()
        .expect_err("missing evidence root should fail the join");
    assert!(matches!(error, GitDoubleError::Io(_)));
    assert!(error.to_string().starts_with("Git double I/O error:"));
    assert!(
        attempt
            .join()
            .expect("push attempt should finish")
            .is_empty()
    );

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}
