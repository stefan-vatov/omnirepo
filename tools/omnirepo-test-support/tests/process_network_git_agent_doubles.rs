// Process, network, Git, and agent double tests stay with their fixture crate.

use std::{
    fs,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use agent_double::{AgentDouble, AgentProtocolViolation};
use git_double::LocalGitRemoteDouble;
use lifecycle_fixture::{FixtureOutcome, FixtureSpec, LifecycleFixture};
use network_double::{HttpDoubleSpec, LocalHttpDouble};
use process_double::{FakeExecutable, ProcessBehavior, ProcessSpec};

use omnirepo_test_support::{
    agent_double, git_double, lifecycle_fixture, network_double, process_double,
};

fn process_capability_or_skip() -> bool {
    if cfg!(unix) {
        true
    } else {
        eprintln!("SKIP process doubles: /bin/sh and Unix signal semantics are unavailable");
        false
    }
}

#[test]
fn executable_publication_fault_and_closed_readiness_are_deterministic() {
    if !process_capability_or_skip() {
        return;
    }

    let mut fixture = LifecycleFixture::create(FixtureSpec::new("publication-red", 411))
        .expect("fixture should be created");
    let published = fixture
        .roots()
        .artifacts()
        .join("publication-red-executable");
    fs::copy(
        std::env::current_exe().expect("test executable should be discoverable"),
        &published,
    )
    .expect("fixture executable should be copied");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&published)
            .expect("fixture executable metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&published, permissions)
            .expect("fixture executable should be runnable");
    }
    fixture
        .track_ephemeral(&published)
        .expect("fixture executable should be tracked");

    // RED: a publication writer that remains open makes Linux reject execve
    // with ETXTBSY (macOS does not enforce that conflict). The GREEN
    // publication path must close this handle before crossing its readiness
    // boundary.
    #[cfg(target_os = "linux")]
    {
        let writer = fs::OpenOptions::new()
            .write(true)
            .open(&published)
            .expect("fixture publication writer should open");
        let spawn = Command::new(&published)
            .arg("--list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let error = spawn.expect_err("open publication writer must fault with ETXTBSY");
        assert_eq!(error.raw_os_error(), Some(26));
        drop(writer);
    }

    let executable =
        fs::read(std::env::current_exe().expect("test executable should be discoverable"))
            .expect("test executable bytes should be readable");
    let ready = fixture
        .publish_executable("publication-green", &executable)
        .expect("fixture publication should close and sync its writer");
    let mut child = Command::new(ready)
        .arg("--list")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("closed publication should cross its readiness boundary");
    child.wait().expect("published executable should be reaped");

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.leaks.is_empty());
}

#[test]
fn fake_executable_has_a_deterministic_barrier_evidence_and_clean_environment() {
    if !process_capability_or_skip() {
        return;
    }
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-tracer", 401))
        .expect("fixture should be created");
    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("tracer", ProcessBehavior::Barrier),
    )
    .expect("fake executable should spawn");

    process
        .wait_for_barrier()
        .expect("child should report the barrier hit");
    process
        .release()
        .expect("barrier release should be explicit");
    let result = process.wait().expect("child should be reaped");

    assert!(result.status.success());
    assert_eq!(
        result.evidence.home,
        fixture.environment().value("HOME").unwrap()
    );
    assert!(result.evidence.ssh_auth_sock_absent);
    assert!(result.evidence.ambient_credentials_absent);
    assert_eq!(result.evidence.barrier, "released");

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.leaks.is_empty());
}

#[test]
fn fake_executable_hang_is_released_without_a_sleep() {
    if !process_capability_or_skip() {
        return;
    }
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-hang", 402))
        .expect("fixture should be created");
    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("hang", ProcessBehavior::Hang),
    )
    .expect("fake executable should spawn");

    assert!(process.try_wait().expect("try_wait should work").is_none());
    process
        .wait_for_barrier()
        .expect("hang should report its deterministic barrier");
    assert!(process.try_wait().expect("try_wait should work").is_none());
    process.release().expect("release should unblock hang");
    assert!(
        process
            .wait()
            .expect("child should be reaped")
            .status
            .success()
    );
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn fake_executable_records_forks_signals_and_bounded_chunked_output() {
    if !process_capability_or_skip() {
        return;
    }
    let mut fork_fixture = LifecycleFixture::create(FixtureSpec::new("process-fork", 403))
        .expect("fixture should be created");
    let mut fork = FakeExecutable::spawn(
        &mut fork_fixture,
        ProcessSpec::new("fork", ProcessBehavior::ForkAndLateWrite),
    )
    .expect("fork double should spawn");
    fork.wait_for_barrier().expect("fork barrier should hit");
    fork.release().expect("fork barrier should release");
    let fork_result = fork.wait().expect("fork tree should be reaped");
    assert!(fork_result.evidence.late_write);
    assert!(
        fork_fixture
            .cleanup(FixtureOutcome::Success)
            .leaks
            .is_empty()
    );

    let mut signal_fixture = LifecycleFixture::create(FixtureSpec::new("process-signal", 404))
        .expect("fixture should be created");
    let mut signal = FakeExecutable::spawn(
        &mut signal_fixture,
        ProcessSpec::new("signal", ProcessBehavior::Signal { number: 15 }),
    )
    .expect("signal double should spawn");
    signal
        .wait_for_barrier()
        .expect("signal barrier should hit");
    signal.release().expect("signal barrier should release");
    let signal_result = signal.wait().expect("signal child should be reaped");
    assert_eq!(signal_result.status.signal, Some(15));
    assert!(
        signal_fixture
            .cleanup(FixtureOutcome::Success)
            .leaks
            .is_empty()
    );

    let mut output_fixture = LifecycleFixture::create(FixtureSpec::new("process-output", 405))
        .expect("fixture should be created");
    let mut output = FakeExecutable::spawn(
        &mut output_fixture,
        ProcessSpec::new(
            "output",
            ProcessBehavior::OversizedChunked {
                chunks: vec!["abcdefgh".to_owned(), "ijklmnop".to_owned()],
            },
        )
        .with_output_limit(5),
    )
    .expect("output double should spawn");
    output
        .wait_for_barrier()
        .expect("output barrier should hit");
    output.release().expect("output barrier should release");
    let output_result = output.wait().expect("output child should be reaped");
    assert_eq!(output_result.stdout.bytes, b"abcde");
    assert!(output_result.stdout.truncated);
    assert!(
        output_fixture
            .cleanup(FixtureOutcome::Success)
            .leaks
            .is_empty()
    );
}

#[test]
fn local_http_double_controls_auth_status_and_evidence() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("http-status", 406))
        .expect("fixture should be created");
    let mut server = LocalHttpDouble::bind(
        &mut fixture,
        "unauthorized",
        HttpDoubleSpec::new(200, b"ok\n".to_vec()).requiring_bearer("fixture-token"),
    )
    .expect("local HTTP server should bind");
    let endpoint = server.endpoint().to_owned();
    let (request_done, request_result) = mpsc::sync_channel(1);
    let client = thread::spawn(move || {
        let result = LocalHttpDouble::send_request(
            &endpoint,
            b"GET /status HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer wrong\r\n\r\n",
        );
        request_done
            .send(())
            .expect("request completion should send");
        result
    });
    let request = server
        .wait_for_request()
        .expect("request barrier should be observable");
    assert!(request.authorization_present);
    assert!(!request.authorization_valid);
    server.respond().expect("401 response should release");
    request_result
        .recv()
        .expect("client completion should be deterministic");
    let raw = client
        .join()
        .expect("client should join")
        .expect("request should succeed");
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 401 Unauthorized"));
    let exchange = server.finish().expect("server should join");
    assert_eq!(exchange.response.status, 401);
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());

    let mut status_fixture = LifecycleFixture::create(FixtureSpec::new("http-404", 407))
        .expect("fixture should be created");
    let mut not_found = LocalHttpDouble::bind(
        &mut status_fixture,
        "not-found",
        HttpDoubleSpec::new(404, b"missing\n".to_vec()),
    )
    .expect("404 server should bind");
    let endpoint = not_found.endpoint().to_owned();
    let client = thread::spawn(move || {
        LocalHttpDouble::send_request(
            &endpoint,
            b"GET /missing HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
    });
    let request = not_found
        .wait_for_request()
        .expect("404 request should be observable");
    assert_eq!(request.path, "/missing");
    not_found.respond().expect("404 response should release");
    let raw = client
        .join()
        .expect("404 client should join")
        .expect("request should succeed");
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 404 Not Found"));
    let exchange = not_found.finish().expect("404 server should join");
    assert_eq!(exchange.response.status, 404);
    assert!(
        status_fixture
            .cleanup(FixtureOutcome::Success)
            .leaks
            .is_empty()
    );
}

#[test]
fn local_git_remote_accepts_payload_then_disconnects_without_git_push() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("git-disconnect", 408))
        .expect("fixture should be created");
    let remote = LocalGitRemoteDouble::bind(&mut fixture, "accepted-then-disconnected")
        .expect("local Git remote should bind");
    let attempt = remote
        .begin_attempt(b"update 0000 refs/heads/main\0 report-status\n")
        .expect("local attempt should start");
    let accepted = remote
        .wait_for_accept()
        .expect("remote should expose accepted payload");
    assert!(accepted.accepted);
    assert!(!accepted.disconnected);
    remote.disconnect().expect("disconnect should be explicit");
    let final_evidence = remote.finish().expect("remote should join");
    assert!(final_evidence.disconnected);
    assert_eq!(final_evidence.payload, accepted.payload);
    assert!(attempt.join().expect("push client should join").is_empty());
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn agent_double_rejects_protocol_violations_and_reaps_session() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("agent-protocol", 409))
        .expect("fixture should be created");
    let session = AgentDouble::start(
        &mut fixture,
        "violations",
        vec![
            "{\"kind\":\"result\",\"status\":\"ok\"}".to_owned(),
            "not-json".to_owned(),
            "{\"kind\":\"result\"}".to_owned(),
            "{\"kind\":\"result\",\"status\":\"ok\",\"extra\":\"x\"}".to_owned(),
        ],
    )
    .expect("agent double should start");
    session
        .wait_for_barrier()
        .expect("agent barrier should be observable");
    session.release().expect("agent release should be explicit");
    let evidence = session.join().expect("agent should be reaped");
    assert_eq!(evidence.accepted.len(), 1);
    assert_eq!(evidence.violations.len(), 3);
    assert!(
        evidence
            .violations
            .contains(&AgentProtocolViolation::MalformedJson)
    );
    assert!(
        evidence
            .violations
            .contains(&AgentProtocolViolation::MissingField("status"))
    );
    assert!(
        evidence
            .violations
            .contains(&AgentProtocolViolation::UnexpectedField("extra".to_owned()))
    );
    assert!(evidence.ambient_credentials_absent);
    assert_eq!(evidence.home, fixture.environment().value("HOME").unwrap());
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}
