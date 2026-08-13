// Lifecycle fixture contract tests are owned by the private support crate.

use std::{
    fs, io,
    path::Path,
    process::Command,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use omnirepo_test_support as support;

use support::lifecycle_fixture::{
    AliasKind, Capability, DirtyGitState, FIXTURE_CONTRACT_VERSION, FixtureError, FixtureOutcome,
    FixtureSpec, LifecycleFixture, RootKind,
};

fn fixture(case_id: &str, seed: u64) -> LifecycleFixture {
    LifecycleFixture::create(FixtureSpec::new(case_id, seed)).expect("fixture should be created")
}

#[test]
fn fixture_contract_creates_isolated_authority_roots_and_env() {
    let fixture = fixture("roots", 17);
    let roots = fixture.roots();
    assert!(roots.home().starts_with(roots.root()));
    assert!(roots.machine_config().starts_with(roots.home()));
    assert!(roots.source_config().starts_with(roots.source()));
    assert!(roots.destination_config().starts_with(roots.destination()));
    assert!(roots.runs().starts_with(roots.home()));

    let environment = fixture.environment();
    assert_eq!(
        environment.value("HOME"),
        Some(roots.home().to_str().unwrap())
    );
    assert_eq!(
        environment.value("USERPROFILE"),
        Some(roots.home().to_str().unwrap())
    );
    assert_eq!(environment.value("GIT_CONFIG_NOSYSTEM"), Some("1"));
    assert!(!environment.vars().contains_key("SSH_AUTH_SOCK"));
    assert!(environment.value("PATH").is_some());

    assert!(
        roots
            .resolve(RootKind::Destination, "nested/file.txt")
            .is_ok()
    );
    assert!(roots.resolve(RootKind::Destination, "/etc/passwd").is_err());
    assert!(roots.resolve(RootKind::Destination, "../outside").is_err());
    assert_eq!(fixture.log().snapshot()[0].kind, "fixture.create");
    assert!(
        fixture.log().snapshot()[0]
            .detail
            .contains(&fixture.fixture_id())
    );
}

#[test]
fn seeded_clock_and_ids_replay_identically_and_log_without_wall_clock() {
    let first = fixture("replay", 99);
    let second = fixture("replay", 99);
    let first_clock = first.clock();
    let second_clock = second.clock();
    let first_ids = first.identities();
    let second_ids = second.identities();

    assert_eq!(first_clock.now(), 0);
    assert_eq!(second_clock.now(), 0);
    assert_eq!(first_ids.run_id(), second_ids.run_id());
    assert_eq!(first_ids.next("lease"), second_ids.next("lease"));
    assert_eq!(first_clock.advance(Duration::from_millis(25)), 25);
    assert_eq!(second_clock.advance(Duration::from_millis(25)), 25);
    assert_eq!(first_clock.now(), second_clock.now());
    first.record("test.replay", format!("run={}", first_ids.run_id()));
    second.record("test.replay", format!("run={}", second_ids.run_id()));
    assert_eq!(
        first
            .log()
            .snapshot()
            .into_iter()
            .map(|event| (event.sequence, event.logical_time, event.kind, event.detail))
            .collect::<Vec<_>>(),
        second
            .log()
            .snapshot()
            .into_iter()
            .map(|event| (event.sequence, event.logical_time, event.kind, event.detail))
            .collect::<Vec<_>>()
    );
}

#[test]
fn run_id_collision_is_explicit_and_deterministic() {
    let fixture = fixture("run-id-collision", 123);
    let identities = fixture.identities();
    identities.force_collision("run", "run-fixed");
    assert_eq!(identities.run_id(), "run-fixed");
    assert_eq!(identities.run_id(), "run-fixed");
    assert_eq!(
        identities.next("other"),
        DeterministicIdentityForTest::expected(123, "other", 1)
    );
}

struct DeterministicIdentityForTest;

impl DeterministicIdentityForTest {
    fn expected(seed: u64, namespace: &str, occurrence: u64) -> String {
        let first = stable_hash(seed, namespace, occurrence);
        let second = stable_hash(seed ^ u64::MAX, namespace, occurrence);
        format!("{namespace}-{first:016x}{second:016x}")
    }
}

fn stable_hash(seed: u64, namespace: &str, occurrence: u64) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64 ^ seed;
    for byte in namespace
        .as_bytes()
        .iter()
        .copied()
        .chain(occurrence.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[test]
fn parallel_fixtures_do_not_share_roots_or_event_sequences() {
    let start = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|index| {
            let start = Arc::clone(&start);
            thread::spawn(move || {
                let mut fixture = fixture("parallel", index);
                start.wait();
                let marker = fixture.roots().artifacts().join(format!("case-{index}"));
                fs::write(&marker, index.to_string()).unwrap();
                fixture.track_ephemeral(&marker).unwrap();
                fixture.record("test.parallel", format!("index={index}"));
                let root = fixture.roots().root().to_path_buf();
                let events = fixture.log().snapshot();
                let report = fixture.cleanup(FixtureOutcome::Success);
                (root, marker, events, report)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("parallel fixture should finish"))
        .collect::<Vec<_>>();
    for (index, (root, marker, events, report)) in results.into_iter().enumerate() {
        assert!(!root.exists());
        assert!(!marker.exists());
        assert_eq!(events[0].sequence, 1);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        assert_eq!(
            index,
            marker.file_name().unwrap().to_str().unwrap()[5..]
                .parse::<usize>()
                .unwrap()
        );
    }
}

#[test]
fn rejected_peer_yields_failure_while_independent_peer_completes() {
    let failed_fixture = fixture("peer-failure", 701);
    let mut successful_fixture = fixture("peer-success", 702);
    let failed_root = failed_fixture.roots().root().to_path_buf();
    let successful_root = successful_fixture.roots().root().to_path_buf();
    let successful_marker = successful_fixture
        .roots()
        .artifacts()
        .join("peer-success.marker");
    assert!(successful_marker.starts_with(&successful_root));
    let successful_marker_for_thread = successful_marker.clone();
    let start = Arc::new(Barrier::new(3));
    let failure_observed = Arc::new(Barrier::new(2));

    let failed_start = Arc::clone(&start);
    let failed_gate = Arc::clone(&failure_observed);
    let failed = thread::spawn(move || {
        failed_start.wait();
        let rejected = failed_fixture
            .roots()
            .resolve(RootKind::Destination, "../outside")
            .expect_err("the failed peer must reject traversal");
        assert!(matches!(rejected, FixtureError::InvalidPath { .. }));
        failed_fixture.record("repository.failure", format!("rejected={rejected}"));
        failed_gate.wait();
        let events = failed_fixture.log().snapshot();
        let report = failed_fixture.cleanup(FixtureOutcome::Failure);
        (events, report)
    });

    let successful_start = Arc::clone(&start);
    let successful_gate = Arc::clone(&failure_observed);
    let successful = thread::spawn(move || {
        successful_start.wait();
        successful_gate.wait();
        fs::write(&successful_marker_for_thread, b"peer-success\n")
            .expect("successful peer writes locally");
        successful_fixture
            .track_ephemeral(&successful_marker_for_thread)
            .expect("successful marker is contained");
        successful_fixture.record("repository.success", "peer continued after sibling failure");
        let events = successful_fixture.log().snapshot();
        let report = successful_fixture.cleanup(FixtureOutcome::Success);
        (events, report)
    });

    start.wait();
    let (failed_events, failed_report) = failed.join().expect("failed peer completes");
    let (successful_events, successful_report) =
        successful.join().expect("successful peer completes");

    assert!(
        failed_report.retained,
        "failed peer retains evidence for reporting"
    );
    assert!(failed_report.root.exists());
    assert!(
        failed_events
            .iter()
            .any(|event| event.kind == "repository.failure")
    );
    assert!(
        successful_report.removed,
        "successful peer completes its cleanup"
    );
    assert!(!successful_report.root.exists());
    assert!(!successful_marker.exists());
    assert!(
        successful_events
            .iter()
            .any(|event| event.kind == "repository.success")
    );
    assert_ne!(failed_root, successful_root);
    assert!(!failed_root.join("artifacts/peer-success.marker").exists());

    fs::remove_dir_all(failed_report.root).expect("remove retained failed-peer evidence");
}

#[test]
fn aliases_permissions_and_special_files_are_contained_or_explicitly_unsupported() {
    let mut fixture = fixture("filesystem-capabilities", 7);
    let target = fixture.roots().artifacts().join("target.txt");
    fs::write(&target, b"exact\n").unwrap();

    let symlink = fixture.roots().artifacts().join("symlink.txt");
    match fixture.create_alias(AliasKind::Symlink, &target, &symlink) {
        Ok(alias_identity) => {
            let target_identity = fixture.roots().identity(&target).unwrap();
            assert!(alias_identity.same_object(&target_identity));
        }
        Err(support::lifecycle_fixture::FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::Symlink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected symlink result: {error}"),
    }

    let hardlink = fixture.roots().artifacts().join("hardlink.txt");
    match fixture.create_alias(AliasKind::HardLink, &target, &hardlink) {
        Ok(alias_identity) => {
            let target_identity = fixture.roots().identity(&target).unwrap();
            assert!(alias_identity.same_object(&target_identity));
        }
        Err(support::lifecycle_fixture::FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::HardLink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected hard-link result: {error}"),
    }

    match fixture.set_mode(&target, 0o640) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                    0o640
                );
            }
        }
        Err(support::lifecycle_fixture::FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::UnixPermissions);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected permissions result: {error}"),
    }

    let fifo = fixture.roots().artifacts().join("events.fifo");
    match fixture.create_fifo(&fifo) {
        Ok(path) => {
            assert!(path.exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                assert!(fs::metadata(path).unwrap().file_type().is_fifo());
            }
        }
        Err(support::lifecycle_fixture::FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::Fifo);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected FIFO result: {error}"),
    }

    assert!(fixture.roots().identity(&target).is_ok());
    assert!(fixture.roots().identity(Path::new("/etc/passwd")).is_err());
}

#[test]
fn dirty_git_fixture_is_local_and_has_explicit_state() {
    let mut fixture = fixture("dirty-git", 31);
    if let Err(support::lifecycle_fixture::FixtureError::Unsupported(capability)) =
        fixture.require(Capability::Git)
    {
        fixture.record("capability.unsupported", capability.to_string());
        return;
    }
    for (state, expected) in [
        (DirtyGitState::Clean, ""),
        (DirtyGitState::Untracked, "?? untracked.txt\n"),
        (DirtyGitState::Modified, " M tracked.txt\n"),
        (DirtyGitState::Staged, "M  tracked.txt\n"),
    ] {
        let root = fixture.roots().artifacts().join(format!("git-{state:?}"));
        let snapshot = fixture.create_git_repository(&root, state).unwrap();
        assert_eq!(snapshot.porcelain, expected);
        assert!(snapshot.root.starts_with(fixture.roots().root()));
    }
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
}

#[test]
fn cleanup_retains_expected_residue_and_reports_unexpected_ephemeral_leaks() {
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new("cleanup", 88).retain_always()).unwrap();
    let expected = fixture.roots().artifacts().join("expected.tmp");
    let leak = fixture.roots().artifacts().join("leak.tmp");
    fs::write(&expected, b"expected").unwrap();
    fs::write(&leak, b"leak").unwrap();
    fixture.track_ephemeral(&expected).unwrap();
    fixture.track_ephemeral(&leak).unwrap();
    fixture.expect_residue(&expected).unwrap();
    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    assert!(!report.removed);
    assert_eq!(report.expected_residue, vec![expected]);
    assert_eq!(report.leaks, vec![leak]);
    assert!(report.root.exists());
    fs::remove_dir_all(report.root).unwrap();
}

#[test]
fn barrier_fixture_requires_explicit_release() {
    let barrier = support::lifecycle_fixture::DeterministicBarrier::new();
    let worker_barrier = barrier.clone();
    let worker = thread::spawn(move || worker_barrier.hit());
    barrier.wait_for_hit().unwrap();
    barrier.release().unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn named_faults_and_barriers_are_exact_and_report_unhit_rules() {
    let fixture = fixture("faults", 5);
    let faults = fixture.faults();
    let point = support::lifecycle_fixture::FaultPoint::new(
        "file.transaction",
        "content_written",
        "worker-1",
        1,
    );
    faults
        .arm(
            point.clone(),
            support::lifecycle_fixture::FaultAction::PartialWrite {
                requested_bytes: 8,
                delivered_bytes: 3,
            },
        )
        .unwrap();
    assert_eq!(
        faults.hit(&point),
        Some(support::lifecycle_fixture::FaultAction::PartialWrite {
            requested_bytes: 8,
            delivered_bytes: 3,
        })
    );
    faults.assert_consumed().unwrap();

    let barriers = fixture.barriers();
    let barrier = barriers.arm("file-write").unwrap();
    let worker = thread::spawn(move || barrier.hit());
    barriers.wait_for_hit("file-write").unwrap();
    barriers.release("file-write").unwrap();
    worker.join().unwrap().unwrap();

    let unhit = support::lifecycle_fixture::FaultController::default();
    unhit
        .arm(
            support::lifecycle_fixture::FaultPoint::new("push", "before_send", "worker-1", 1),
            support::lifecycle_fixture::FaultAction::AcceptThenDisconnect,
        )
        .unwrap();
    assert!(unhit.assert_consumed().is_err());
}

#[test]
fn fixture_contract_version_is_stable() {
    assert_eq!(FIXTURE_CONTRACT_VERSION, "lifecycle-fixtures/v1");
}

#[test]
fn deterministic_getters_cover_every_root_and_sanitized_environment() {
    let fixture = fixture("deterministic-getters", 1701);
    let roots = fixture.roots();
    let root_cases = [
        (RootKind::Root, roots.root()),
        (RootKind::Home, roots.home()),
        (RootKind::MachineConfig, roots.machine_config_root()),
        (RootKind::Source, roots.source()),
        (RootKind::SourceConfig, roots.source_config_root()),
        (RootKind::SourceSnapshot, roots.source_snapshot()),
        (RootKind::Destination, roots.destination()),
        (RootKind::Runs, roots.runs()),
        (RootKind::Artifacts, roots.artifacts()),
        (RootKind::Remote, roots.remote()),
    ];
    for (kind, base) in root_cases {
        assert_eq!(
            roots.resolve(kind, "case-marker.txt").unwrap(),
            base.join("case-marker.txt"),
            "root kind {kind:?} must resolve beneath its declared base"
        );
    }
    assert!(roots.contains(roots.root()));
    assert!(!roots.contains(Path::new("/etc/passwd")));

    let mut command = Command::new("env");
    command.env("SHOULD_BE_CLEARED", "ambient-value");
    fixture.environment().apply(&mut command);
    let output = command
        .output()
        .expect("sanitized environment command should run");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("environment output is UTF-8");
    assert!(stdout.contains(&format!("HOME={}\n", roots.home().display())));
    assert!(stdout.contains("GIT_CONFIG_NOSYSTEM=1\n"));
    assert!(!stdout.contains("SHOULD_BE_CLEARED="));
    if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
        assert!(
            stdout.contains(&format!("LLVM_PROFILE_FILE={profile}\n")),
            "coverage profile path must survive fixture env sanitization"
        );
    }

    let spec = FixtureSpec::new("spec-getters", 1702);
    assert_eq!(spec.case_id, "spec-getters");
    assert_eq!(spec.seed, 1702);
    assert_eq!(
        spec.cleanup,
        support::lifecycle_fixture::CleanupMode::RemoveOnSuccessRetainOnFailure
    );
    assert_eq!(
        spec.clone().retain_always().cleanup,
        support::lifecycle_fixture::CleanupMode::RetainAlways
    );

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn root_resolver_rejects_parent_traversal_and_symlinked_escape_before_effects() {
    let mut fixture = fixture("root-confinement", 18);
    let artifacts = fixture.roots().artifacts().to_path_buf();
    assert!(
        fixture
            .roots()
            .resolve(RootKind::Artifacts, "nested/file.txt")
            .is_ok()
    );

    let parent_traversal = artifacts.join("../outside");
    assert!(matches!(
        fixture.track_ephemeral(&parent_traversal),
        Err(FixtureError::InvalidPath { .. })
    ));

    #[cfg(unix)]
    {
        let link = artifacts.join("outside-link");
        std::os::unix::fs::symlink("/etc", &link).expect("escape symlink should be created");
        let escaped_child = artifacts.join("outside-link/payload");
        assert!(matches!(
            fixture
                .roots()
                .resolve(RootKind::Artifacts, "outside-link/payload"),
            Err(FixtureError::EscapesRoot(_))
        ));
        assert!(matches!(
            fixture.track_ephemeral(&escaped_child),
            Err(FixtureError::EscapesRoot(_))
        ));
        assert!(matches!(
            fixture.expect_residue(&escaped_child),
            Err(FixtureError::EscapesRoot(_))
        ));
    }

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn root_identity_rejects_missing_paths_and_symlink_escape() {
    let mut fixture = fixture("identity-escape", 1703);
    let missing = fixture.roots().artifacts().join("missing");
    assert!(matches!(
        fixture.roots().identity(&missing),
        Err(FixtureError::Io(error)) if error.kind() == io::ErrorKind::NotFound
    ));

    let escape = fixture.roots().artifacts().join("escape.alias");
    match fixture.require(Capability::Symlink) {
        Ok(()) => {
            #[cfg(unix)]
            std::os::unix::fs::symlink("/etc/passwd", &escape)
                .expect("fixture should create the escape alias");
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(
                "C:\\Windows\\System32\\drivers\\etc\\hosts",
                &escape,
            )
            .expect("fixture should create the escape alias");
            assert!(matches!(
                fixture.track_ephemeral(&escape),
                Err(FixtureError::EscapesRoot(_))
            ));
            match fixture.roots().identity(&escape) {
                Err(FixtureError::EscapesRoot(path)) => {
                    assert!(!path.starts_with(fixture.roots().root()));
                }
                Err(error) => panic!("unexpected symlink identity result: {error}"),
                Ok(_) => panic!("an alias to an external file must not be accepted"),
            }
        }
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::Symlink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected symlink capability result: {error}"),
    }

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn fixture_error_and_capability_formatting_is_stable() {
    let capabilities = [
        (Capability::Git, "git"),
        (Capability::Symlink, "symlink"),
        (Capability::HardLink, "hard-link"),
        (Capability::Fifo, "fifo"),
        (Capability::UnixPermissions, "unix-permissions"),
    ];
    for (capability, name) in capabilities {
        assert_eq!(capability.to_string(), name);
        let unsupported = support::lifecycle_fixture::UnsupportedCapability {
            capability,
            reason: "test capability reason".to_owned(),
        };
        assert_eq!(
            unsupported.to_string(),
            format!("{name} (test capability reason)")
        );
        assert!(
            FixtureError::Unsupported(unsupported)
                .to_string()
                .contains("unsupported fixture capability")
        );
    }

    let errors = [
        FixtureError::Io(io::Error::new(io::ErrorKind::NotFound, "missing")),
        FixtureError::InvalidPath {
            path: "../escape".to_owned(),
            reason: "parent traversal is not accepted",
        },
        FixtureError::EscapesRoot(Path::new("/etc/passwd").to_path_buf()),
        FixtureError::Command {
            program: "mkfifo".to_owned(),
            status: "exit status: 1".to_owned(),
            stderr: "already exists".to_owned(),
        },
        FixtureError::Invariant("bad state".to_owned()),
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn deterministic_clock_default_set_and_zero_advance_are_replayable() {
    let clock = support::lifecycle_fixture::DeterministicClock::default();
    assert_eq!(clock.now(), 0);
    clock.set(42);
    assert_eq!(clock.now(), 42);
    assert_eq!(clock.advance(Duration::ZERO), 42);
    let clone = clock.clone();
    assert_eq!(clone.advance(Duration::from_millis(8)), 50);
    assert_eq!(clock.now(), 50);
}

#[test]
fn barriers_and_controllers_reject_misuse_without_wall_clock_waits() {
    let barrier = support::lifecycle_fixture::DeterministicBarrier::new();
    assert!(matches!(
        barrier.release(),
        Err(FixtureError::Invariant(message)) if message == "barrier was not hit"
    ));
    let worker_barrier = barrier.clone();
    let worker = thread::spawn(move || worker_barrier.hit());
    barrier.wait_for_hit().unwrap();
    assert!(matches!(
        barrier.hit(),
        Err(FixtureError::Invariant(message)) if message == "barrier was hit more than once"
    ));
    barrier.release().unwrap();
    worker.join().unwrap().unwrap();
    barrier.wait_for_hit().unwrap();

    let aborted = support::lifecycle_fixture::DeterministicBarrier::new();
    aborted.abort();
    assert!(matches!(
        aborted.wait_for_hit(),
        Err(FixtureError::Invariant(message)) if message == "barrier was aborted"
    ));

    let controller = support::lifecycle_fixture::BarrierController::default();
    for result in [
        controller.wait_for_hit("unknown"),
        controller.release("unknown"),
        controller.abort("unknown"),
    ] {
        assert!(
            matches!(result, Err(FixtureError::Invariant(message)) if message.contains("barrier is not armed"))
        );
    }
    let controlled = controller.arm("controlled").unwrap();
    assert!(matches!(
        controller.arm("controlled"),
        Err(FixtureError::Invariant(message)) if message == "barrier already armed: controlled"
    ));
    let controlled_worker = thread::spawn(move || controlled.hit());
    controller.wait_for_hit("controlled").unwrap();
    controller.release("controlled").unwrap();
    controlled_worker.join().unwrap().unwrap();

    controller.arm("aborted").unwrap();
    controller.abort("aborted").unwrap();
    assert!(matches!(
        controller.wait_for_hit("aborted"),
        Err(FixtureError::Invariant(message)) if message == "barrier was aborted"
    ));
    assert!(matches!(
        controller.release("aborted"),
        Err(FixtureError::Invariant(message)) if message == "barrier was not hit"
    ));
}

#[test]
fn duplicate_faults_and_unhit_rules_are_explicit() {
    let faults = support::lifecycle_fixture::FaultController::default();
    let point = support::lifecycle_fixture::FaultPoint::new("duplicate", "phase", "actor", 1);
    let action = support::lifecycle_fixture::FaultAction::ReturnError("injected".to_owned());
    faults.arm(point.clone(), action.clone()).unwrap();
    assert!(matches!(
        faults.arm(point.clone(), action.clone()),
        Err(FixtureError::Invariant(message)) if message.contains("fault point already armed")
    ));
    let other = support::lifecycle_fixture::FaultPoint::new("other", "phase", "actor", 1);
    assert!(faults.hit(&other).is_none());
    assert!(faults.assert_consumed().is_err());
    assert_eq!(faults.hit(&point), Some(action.clone()));
    assert_eq!(faults.hit(&point), Some(action));
    faults.assert_consumed().unwrap();
    support::lifecycle_fixture::FaultController::default()
        .assert_consumed()
        .unwrap();
}

#[test]
fn executable_identity_rejection_and_duplicate_publication_are_deterministic() {
    let mut fixture = fixture("publication-edges", 1704);
    for invalid in ["", ".", "..", "a/b", r"a\b", "line\nbreak", "line\rbreak"] {
        assert!(
            matches!(
                fixture.publish_executable(invalid, b"#!/bin/sh\n"),
                Err(FixtureError::InvalidPath { .. })
            ),
            "identity {invalid:?} must be rejected"
        );
    }
    if let Err(FixtureError::Unsupported(capability)) = fixture.require(Capability::UnixPermissions)
    {
        fixture.record("capability.unsupported", capability.to_string());
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        return;
    }

    let staged = fixture.roots().artifacts().join("runner.script.tmp");
    fs::write(&staged, b"occupied").unwrap();
    fixture.track_ephemeral(&staged).unwrap();
    match fixture.publish_executable("runner", b"#!/bin/sh\nexit 0\n") {
        Err(FixtureError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::AlreadyExists),
        Err(error) => panic!("unexpected duplicate staging result: {error}"),
        Ok(path) => panic!(
            "occupied temporary publication path was replaced: {}",
            path.display()
        ),
    }
    fs::remove_file(&staged).unwrap();
    let path = fixture
        .publish_executable("runner", b"#!/bin/sh\nexit 0\n")
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"#!/bin/sh\nexit 0\n");
    let replacement = fixture
        .publish_executable("runner", b"#!/bin/sh\nexit 7\n")
        .unwrap();
    assert_eq!(replacement, path);
    assert_eq!(fs::read(&replacement).unwrap(), b"#!/bin/sh\nexit 7\n");
    assert!(
        fixture
            .log()
            .snapshot()
            .iter()
            .any(|event| event.kind == "fixture.executable.publish")
    );

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn alias_failures_reject_external_and_missing_targets_and_duplicate_aliases() {
    let mut fixture = fixture("alias-failures", 1705);
    let missing = fixture.roots().artifacts().join("missing.target");
    let missing_alias = fixture.roots().artifacts().join("missing.alias");
    match fixture.create_alias(AliasKind::HardLink, &missing, &missing_alias) {
        Err(FixtureError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::HardLink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected missing-target result: {error}"),
        Ok(_) => panic!("missing alias target must fail"),
    }

    let target = fixture.roots().artifacts().join("target.txt");
    fs::write(&target, b"target\n").unwrap();
    let inside_alias = fixture.roots().artifacts().join("inside.alias");
    match fixture.create_alias(AliasKind::HardLink, &target, &inside_alias) {
        Ok(identity) => {
            assert!(identity.same_object(&fixture.roots().identity(&target).unwrap()));
            match fixture.create_alias(AliasKind::HardLink, &target, &inside_alias) {
                Err(FixtureError::Io(error)) => {
                    assert!(matches!(
                        error.kind(),
                        io::ErrorKind::AlreadyExists | io::ErrorKind::Other
                    ));
                }
                Err(error) => panic!("unexpected duplicate hard-link result: {error}"),
                Ok(_) => panic!("duplicate hard-link alias must fail"),
            }
        }
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::HardLink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected hard-link result: {error}"),
    }

    let external_alias = fixture.roots().artifacts().join("external.alias");
    match fixture.create_alias(
        AliasKind::Symlink,
        Path::new("/etc/passwd"),
        &external_alias,
    ) {
        Err(FixtureError::EscapesRoot(path)) => assert_eq!(path, external_alias),
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::Symlink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected external-target result: {error}"),
        Ok(_) => panic!("external alias target must fail"),
    }
    let outside_alias = Path::new("/tmp/omnirepo-outside-alias");
    match fixture.create_alias(AliasKind::Symlink, &target, outside_alias) {
        Err(FixtureError::EscapesRoot(path)) => assert_eq!(path, outside_alias),
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::Symlink);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected external-alias result: {error}"),
        Ok(_) => panic!("external alias path must fail"),
    }

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn fifo_command_failure_is_captured_and_capability_is_classified() {
    let mut fixture = fixture("fifo-failure", 1706);
    let path = fixture.roots().artifacts().join("already-exists.fifo");
    fs::write(&path, b"not-a-fifo").unwrap();
    match fixture.require(Capability::Fifo) {
        Ok(()) => match fixture.create_fifo(&path) {
            Err(FixtureError::Command {
                program,
                status,
                stderr,
            }) => {
                assert_eq!(program, "mkfifo");
                assert!(!status.is_empty());
                assert!(!stderr.is_empty());
            }
            Err(error) => panic!("unexpected FIFO failure: {error}"),
            Ok(_) => panic!("mkfifo must reject an existing regular file"),
        },
        Err(FixtureError::Unsupported(capability)) => {
            assert_eq!(capability.capability, Capability::Fifo);
            fixture.record("capability.unsupported", capability.to_string());
        }
        Err(error) => panic!("unexpected FIFO capability result: {error}"),
    }
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn external_tracking_is_rejected_and_owned_failure_residue_is_reported() {
    let mut retained_fixture = fixture("owned-cleanup", 1707);
    let external = Path::new("/etc/passwd").to_path_buf();
    assert!(matches!(
        retained_fixture.track_ephemeral(&external),
        Err(FixtureError::EscapesRoot(path)) if path == external
    ));
    assert!(matches!(
        retained_fixture.expect_residue(&external),
        Err(FixtureError::EscapesRoot(path)) if path == external
    ));

    let owned = retained_fixture.roots().artifacts().join("owned.tmp");
    fs::write(&owned, b"owned evidence").unwrap();
    retained_fixture.track_ephemeral(&owned).unwrap();
    let retained = retained_fixture.cleanup(FixtureOutcome::Failure);
    assert!(retained.retained);
    assert!(!retained.removed);
    assert_eq!(retained.leaks, vec![owned]);
    assert!(retained.root.exists());
    fs::remove_dir_all(retained.root).unwrap();

    let mut success = fixture("owned-cleanup-success", 1708);
    let removed = success.roots().artifacts().join("owned.tmp");
    fs::write(&removed, b"owned").unwrap();
    success.track_ephemeral(&removed).unwrap();
    let report = success.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    assert!(!removed.exists());
}
