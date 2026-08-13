// Fixture-layer invariants are owned and exercised by the private support crate.

use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use omnirepo_test_support::{
    agent_double, git_double, lifecycle_fixture, network_double, process_double,
};

use agent_double::{AgentDouble, AgentProtocolViolation};
use git_double::LocalGitRemoteDouble;
use lifecycle_fixture::{
    AliasKind, Capability, DirtyGitState, FaultAction, FaultPoint, FixtureError, FixtureOutcome,
    FixtureSpec, LifecycleFixture, RootKind,
};
use network_double::{HttpDoubleSpec, LocalHttpDouble};
use process_double::{FakeExecutable, ProcessBehavior, ProcessSpec};

#[derive(Debug, Clone, Copy)]
enum TemporaryMutation {
    MissingFaultDelivery,
    Restored,
}

fn run_fault_delivery_self_test(mutation: TemporaryMutation) -> Result<(), FixtureError> {
    let fixture = LifecycleFixture::create(FixtureSpec::new("self-test-tracer", 7101))?;
    let fault = FaultPoint::new("self-test", "before-effect", "self-test", 1);
    fixture.faults().arm(
        fault.clone(),
        FaultAction::ReturnError("injected".to_owned()),
    )?;
    if matches!(mutation, TemporaryMutation::Restored) {
        let _ = fixture.faults().hit(&fault);
    }
    let result = fixture.faults().assert_consumed();
    let report = fixture.cleanup(if result.is_ok() {
        FixtureOutcome::Success
    } else {
        FixtureOutcome::Failure
    });
    if result.is_err() {
        assert!(report.retained);
        assert_eq!(report.leaks.len(), 0);
        fs::remove_dir_all(report.root).expect("retained mutation evidence should be removable");
    } else {
        assert!(report.removed);
        assert!(report.leaks.is_empty());
    }
    result
}

#[test]
fn temporary_missing_fault_mutation_is_rejected() {
    let result = run_fault_delivery_self_test(TemporaryMutation::MissingFaultDelivery);
    assert!(
        result.is_err(),
        "self-test must reject missing fault delivery"
    );
    assert!(run_fault_delivery_self_test(TemporaryMutation::Restored).is_ok());
}

fn replay_evidence(case_id: &str, seed: u64) -> Vec<(u64, u64, String, String)> {
    let fixture = LifecycleFixture::create(FixtureSpec::new(case_id, seed))
        .expect("replay fixture should be created");
    let clock = fixture.clock();
    let identities = fixture.identities();
    let first_run = identities.run_id();
    let first_lease = identities.next("lease");
    clock.advance(Duration::from_millis(17));
    fixture.record(
        "self-test.replay",
        format!("run={first_run};lease={first_lease}"),
    );
    let events = fixture
        .log()
        .snapshot()
        .into_iter()
        .map(|event| (event.sequence, event.logical_time, event.kind, event.detail))
        .collect();
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    events
}

#[test]
fn fixed_seed_replay_reproduces_clock_ids_and_event_evidence() {
    assert_eq!(
        replay_evidence("fixed-seed-replay", 7102),
        replay_evidence("fixed-seed-replay", 7102)
    );
}

#[test]
fn parallel_cases_have_independent_roots_and_no_cross_case_state() {
    let start = Arc::new(Barrier::new(4));
    let handles = (0..4)
        .map(|index| {
            let start = Arc::clone(&start);
            thread::spawn(move || {
                let mut fixture = LifecycleFixture::create(FixtureSpec::new(
                    format!("parallel-self-test-{index}"),
                    7110 + index,
                ))
                .expect("parallel fixture should be created");
                start.wait();
                let marker = fixture.roots().artifacts().join("case.marker");
                fs::write(&marker, index.to_string()).expect("case marker should be written");
                fixture
                    .track_ephemeral(&marker)
                    .expect("case marker should be contained");
                fixture.record("self-test.parallel", format!("case={index}"));
                let root = fixture.roots().root().to_path_buf();
                let events = fixture.log().snapshot();
                let report = fixture.cleanup(FixtureOutcome::Success);
                (index, root, marker, events, report)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("parallel self-test should join"))
        .collect::<Vec<_>>();
    let roots = results
        .iter()
        .map(|(_, root, _, _, _)| root)
        .collect::<Vec<_>>();
    for (index, root, marker, events, report) in &results {
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        assert!(!root.exists());
        assert!(!marker.exists());
        assert!(events.iter().any(|event| {
            event.kind == "self-test.parallel" && event.detail == format!("case={index}")
        }));
    }
    for (index, root) in roots.iter().enumerate() {
        assert!(roots.iter().skip(index + 1).all(|other| *other != *root));
    }
}

#[test]
fn environment_and_roots_reject_real_home_and_escape_paths() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("environment-roots", 7120))
        .expect("environment fixture should be created");
    let roots = fixture.roots();
    let environment = fixture.environment();
    assert_eq!(environment.value("HOME"), roots.home().to_str());
    assert_eq!(environment.value("USERPROFILE"), roots.home().to_str());
    assert_eq!(environment.value("GIT_CONFIG_NOSYSTEM"), Some("1"));
    assert_eq!(
        environment.value("PATH"),
        Some("/usr/local/bin:/usr/bin:/bin")
    );
    for key in [
        "SSH_AUTH_SOCK",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "GITHUB_TOKEN",
    ] {
        assert!(
            !environment.vars().contains_key(key),
            "ambient key leaked: {key}"
        );
    }
    assert!(roots.resolve(RootKind::Home, "nested/file").is_ok());
    assert!(roots.resolve(RootKind::Destination, "/etc/passwd").is_err());
    assert!(roots.resolve(RootKind::Destination, "../outside").is_err());
    assert!(roots.identity(Path::new("/etc/passwd")).is_err());
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn filesystem_and_git_roots_are_local_and_state_is_explicit() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("filesystem-git-roots", 7130))
        .expect("filesystem fixture should be created");
    let target = fixture.roots().artifacts().join("target.txt");
    fs::write(&target, b"target\n").expect("target should be written");
    fixture
        .track_ephemeral(&target)
        .expect("target should be tracked");
    let alias = fixture.roots().artifacts().join("target.alias");
    match fixture.create_alias(AliasKind::HardLink, &target, &alias) {
        Ok(identity) => {
            assert!(identity.same_object(&fixture.roots().identity(&target).unwrap()));
        }
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::HardLink);
        }
        Err(error) => panic!("unexpected hard-link result: {error}"),
    }
    if fixture.require(Capability::Git).is_ok() {
        let git_root = fixture.roots().artifacts().join("dirty-git");
        let snapshot = fixture
            .create_git_repository(&git_root, DirtyGitState::Modified)
            .expect("local Git repository should be created");
        assert!(snapshot.root.starts_with(fixture.roots().root()));
        assert_eq!(snapshot.porcelain, " M tracked.txt\n");
    }
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn process_double_controls_barrier_signal_descendant_and_output_without_leaks() {
    if !cfg!(unix) {
        eprintln!("SKIP process self-test: Unix executable semantics unavailable");
        return;
    }
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-self-test", 7140))
        .expect("process fixture should be created");
    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("fork", ProcessBehavior::ForkAndLateWrite),
    )
    .expect("process double should spawn");
    process
        .wait_for_barrier()
        .expect("process barrier should be observable");
    process
        .release()
        .expect("process release should be explicit");
    let result = process.wait().expect("process tree should be reaped");
    assert!(result.status.success());
    assert!(result.evidence.late_write);
    assert!(result.evidence.ambient_credentials_absent);

    let mut signal = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("signal", ProcessBehavior::Signal { number: 15 }),
    )
    .expect("signal double should spawn");
    signal
        .wait_for_barrier()
        .expect("signal barrier should be observable");
    signal.release().expect("signal release should be explicit");
    assert_eq!(
        signal
            .wait()
            .expect("signal child should be reaped")
            .status
            .signal,
        Some(15)
    );

    let mut output = FakeExecutable::spawn(
        &mut fixture,
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
        .expect("output barrier should be observable");
    output.release().expect("output release should be explicit");
    let output_result = output.wait().expect("output child should be reaped");
    assert_eq!(output_result.stdout.bytes, b"abcde");
    assert!(output_result.stdout.truncated);

    let mut hanging = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("drop-hang", ProcessBehavior::Hang),
    )
    .expect("hang double should spawn");
    hanging
        .wait_for_barrier()
        .expect("hang barrier should be observable");
    drop(hanging);

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn local_network_and_git_doubles_never_use_an_ambient_service() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-self-test", 7150))
        .expect("network fixture should be created");
    let mut server = LocalHttpDouble::bind(
        &mut fixture,
        "auth",
        HttpDoubleSpec::new(200, b"ok\n".to_vec()).requiring_bearer("fixture-token"),
    )
    .expect("local HTTP double should bind");
    assert!(server.endpoint().starts_with("http://127.0.0.1:"));
    let endpoint = server.endpoint().to_owned();
    let client = thread::spawn(move || {
        LocalHttpDouble::send_request(
            &endpoint,
            b"GET /health HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer fixture-token\r\n\r\n",
        )
    });
    let request = server
        .wait_for_request()
        .expect("local HTTP request should reach barrier");
    assert!(request.authorization_valid);
    server
        .respond()
        .expect("local HTTP response should release");
    assert!(
        String::from_utf8_lossy(
            &client
                .join()
                .expect("HTTP client should join")
                .expect("HTTP request should complete")
        )
        .starts_with("HTTP/1.1 200 OK")
    );
    assert_eq!(
        server
            .finish()
            .expect("HTTP server should join")
            .response
            .status,
        200
    );

    let remote =
        LocalGitRemoteDouble::bind(&mut fixture, "git").expect("local Git remote should bind");
    assert!(remote.endpoint().starts_with("git://127.0.0.1:"));
    let attempt = remote
        .begin_attempt(b"fixture-payload")
        .expect("local Git attempt should start");
    let accepted = remote
        .wait_for_accept()
        .expect("Git remote should accept payload");
    assert!(accepted.accepted);
    remote
        .disconnect()
        .expect("Git disconnect should be explicit");
    let disconnected = remote.finish().expect("Git remote should join");
    assert!(disconnected.disconnected);
    assert!(attempt.join().expect("Git client should join").is_empty());
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn agent_double_controls_barrier_credentials_and_protocol_violations() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("agent-self-test", 7160))
        .expect("agent fixture should be created");
    let session = AgentDouble::start(
        &mut fixture,
        "protocol",
        vec![
            "{\"kind\":\"result\",\"status\":\"ok\"}".to_owned(),
            "not-json".to_owned(),
            "{\"kind\":\"result\"}".to_owned(),
        ],
    )
    .expect("agent double should start");
    session
        .wait_for_barrier()
        .expect("agent barrier should be observable");
    session.release().expect("agent release should be explicit");
    let evidence = session.join().expect("agent session should be reaped");
    assert_eq!(evidence.accepted.len(), 1);
    assert_eq!(evidence.violations.len(), 2);
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
    assert_eq!(evidence.home, fixture.environment().value("HOME").unwrap());
    assert!(evidence.ambient_credentials_absent);
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn pty_capability_is_checked_without_claiming_a_missing_pty_double() {
    let capability = cfg!(unix) && Path::new("/dev/ptmx").exists();
    if !capability {
        eprintln!("SKIP PTY self-test: PTY capability unavailable");
        return;
    }
    eprintln!("SKIP PTY self-test: lifecycle-fixtures/v1 has no PTY double");
}

#[test]
fn cleanup_retains_failure_evidence_and_reports_unexpected_leaks() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("cleanup-self-test", 7170))
        .expect("cleanup fixture should be created");
    let expected = fixture.roots().artifacts().join("expected.evidence");
    let leak = fixture.roots().artifacts().join("unexpected.evidence");
    fs::write(&expected, b"expected\n").expect("expected residue should be written");
    fs::write(&leak, b"leak\n").expect("leak should be written");
    fixture
        .track_ephemeral(&expected)
        .expect("expected residue should be contained");
    fixture
        .track_ephemeral(&leak)
        .expect("leak should be contained");
    fixture
        .expect_residue(&expected)
        .expect("expected residue should be declared");
    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    assert_eq!(report.expected_residue, vec![expected]);
    assert_eq!(report.leaks, vec![leak]);
    assert!(report.root.exists());
    fs::remove_dir_all(report.root).expect("failure evidence should be removable");
}
