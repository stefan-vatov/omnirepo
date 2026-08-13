//! Composed-suite isolation and anti-flake stress checks.
//!
//! This target consumes the public fixture, recovery, process, and evidence
//! seams. It does not duplicate their implementation or the acceptance
//! journey matrix.

use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use omnirepo_test_support::e2e_runner_crimson_coast::{
    E2eRunner, ExpectedEffects, ExpectedFile, FixtureBinarySpec, RunnerCase,
};
use omnirepo_test_support::lifecycle_fixture::{
    AliasKind, Capability, DeterministicBarrier, DirtyGitState, FixtureError, FixtureOutcome,
    FixtureSpec, LifecycleFixture, RootKind, UnsupportedCapability,
};
use omnirepo_test_support::network_double::{HttpDoubleSpec, LocalHttpDouble};
use omnirepo_test_support::process_double::{FakeExecutable, ProcessBehavior, ProcessSpec};
use omnirepo_test_support::recovery_control::{
    CleanupRaceControl, ConcurrentRunControl, CrashSpec, CrashableParent, JournalWriterControl,
    RetainedState, WriterError, WriterFault,
};
use omnirepo_test_support::test_evidence::{
    ArtifactReference, ArtifactStore, DiagnosticRedactor, EventKind, EventRecorder, EvidenceError,
    Outcome, SourcePlanConfig, TestIdentity, execute_case,
};

fn replay_evidence(seed: u64) -> Vec<(u64, u64, String, String)> {
    let fixture = LifecycleFixture::create(FixtureSpec::new("isolation-replay", seed))
        .expect("fixture should be created");
    let clock = fixture.clock();
    let identities = fixture.identities();
    let run_id = identities.run_id();
    let lease_id = identities.next("lease");
    let capabilities = [
        Capability::Git,
        Capability::Symlink,
        Capability::HardLink,
        Capability::Fifo,
        Capability::UnixPermissions,
    ]
    .into_iter()
    .map(|capability| {
        let status = match fixture.require(capability) {
            Ok(()) => "supported",
            Err(FixtureError::Unsupported(_)) => "unsupported",
            Err(FixtureError::Io(_)) => "io-error",
            Err(FixtureError::InvalidPath { .. })
            | Err(FixtureError::EscapesRoot(_))
            | Err(FixtureError::Command { .. })
            | Err(FixtureError::Invariant(_)) => "fixture-error",
        };
        format!("{capability}={status}")
    })
    .collect::<Vec<_>>()
    .join(",");
    clock.advance(Duration::from_millis(23));
    fixture.record(
        "stress.replay",
        format!("run={run_id};lease={lease_id};capabilities={capabilities}"),
    );
    let evidence = fixture
        .log()
        .snapshot()
        .into_iter()
        .map(|event| (event.sequence, event.logical_time, event.kind, event.detail))
        .collect();
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    evidence
}

#[test]
fn same_seed_replay_is_byte_equivalent_before_suite_expansion() {
    assert_eq!(
        replay_evidence(91_001),
        replay_evidence(91_001),
        "same case and seed must produce equivalent structured evidence"
    );
}

#[test]
fn composed_hard_link_attack_is_reported_by_public_e2e_containment() {
    let mut capability_fixture = LifecycleFixture::create(FixtureSpec::new(
        "hard-link-attack-capability-probe",
        91_001,
    ))
    .expect("hard-link capability fixture should be created");
    if !require_or_record_capability_skip(
        &mut capability_fixture,
        "hard-link-attack-capability-probe",
        91_001,
        &[
            Capability::UnixPermissions,
            Capability::HardLink,
            Capability::Git,
        ],
    ) {
        let report = capability_fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }
    let report = capability_fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    let case = RunnerCase::new(
        "hard-link-attack-composed",
        FixtureBinarySpec::shell(
            "hard-link-attack",
            "#!/bin/sh\nln \"$OMNIREPO_E2E_OUTSIDE_CANARY/sentinel\" \"$OMNIREPO_E2E_EFFECTS_ROOT/hard-link\"\n",
        ),
    )
    .expect("hard-link attack case should be valid")
    .expected(ExpectedEffects::success());
    let error = E2eRunner::new()
        .run(case)
        .expect_err("hard-link effects must fail closed");
    let report = error
        .report()
        .expect("hard-link attack should retain evidence");
    assert!(
        !report.containment.hard_link_paths.is_empty(),
        "hard-link containment evidence must identify the attack path"
    );
    fs::remove_dir_all(&report.root).expect("remove retained hard-link evidence");
}

#[test]
fn public_alias_and_e2e_attack_paths_are_barriered_and_fail_closed() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("alias-attacks", 91_002))
        .expect("alias attack fixture should be created");
    if !require_or_record_capability_skip(
        &mut fixture,
        "alias-attacks",
        91_002,
        &[
            Capability::UnixPermissions,
            Capability::Symlink,
            Capability::HardLink,
            Capability::Git,
        ],
    ) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }

    let artifacts = fixture.roots().artifacts().to_path_buf();
    let target = fixture
        .roots()
        .resolve(RootKind::Artifacts, "alias-target.txt")
        .expect("alias target should be contained");
    let outside = fixture
        .roots()
        .resolve(RootKind::Root, "outside-sentinel.txt")
        .expect("outside sentinel should be fixture-contained");
    fs::write(&target, b"target\n").expect("alias target should be written");
    fs::create_dir_all(&outside).expect("outside sentinel directory should be created");

    let symlink_alias = artifacts.join("symlink-alias");
    fixture
        .create_alias(AliasKind::Symlink, &outside, &symlink_alias)
        .expect("symlink alias should be created");
    let store = ArtifactStore::new(&artifacts).expect("artifact store should be available");
    let symlink_barrier = DeterministicBarrier::new();
    let symlink_worker_barrier = symlink_barrier.clone();
    let symlink_store = store.clone();
    let symlink_worker = thread::spawn(move || {
        symlink_worker_barrier
            .hit()
            .expect("symlink writer should hit the barrier");
        symlink_store
            .write_bytes("symlink-alias/payload", b"escape")
            .expect_err("symlink component must be rejected before writing")
    });
    symlink_barrier
        .wait_for_hit()
        .expect("symlink attack writer should reach the barrier");
    symlink_barrier
        .release()
        .expect("symlink attack barrier should release");
    let symlink_error = symlink_worker
        .join()
        .expect("symlink attack writer should join");
    assert!(matches!(symlink_error, EvidenceError::ArtifactSymlink(_)));
    assert!(!outside.join("payload").exists());

    let hard_link_alias = artifacts.join("hard-link-alias");
    fixture
        .create_alias(AliasKind::HardLink, &target, &hard_link_alias)
        .expect("hard-link alias should be created");
    let hard_link_barrier = DeterministicBarrier::new();
    let hard_link_worker_barrier = hard_link_barrier.clone();
    let hard_link_store = store.clone();
    let hard_link_worker = thread::spawn(move || {
        hard_link_worker_barrier
            .hit()
            .expect("hard-link writer should hit the barrier");
        hard_link_store
            .write_bytes("hard-link-alias", b"overwrite")
            .expect_err("hard-link destination must not be overwritten")
    });
    hard_link_barrier
        .wait_for_hit()
        .expect("hard-link attack writer should reach the barrier");
    hard_link_barrier
        .release()
        .expect("hard-link attack barrier should release");
    let hard_link_error = hard_link_worker
        .join()
        .expect("hard-link attack writer should join");
    assert!(
        matches!(hard_link_error, EvidenceError::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(
        fs::read(&target).expect("hard-link target should remain readable"),
        b"target\n"
    );

    let runner = E2eRunner::new();
    let symlink_case = RunnerCase::new(
        "public-symlink-attack",
        FixtureBinarySpec::shell(
            "public-symlink-attack",
            "#!/bin/sh\nln -s \"$OMNIREPO_E2E_OUTSIDE_CANARY\" \"$OMNIREPO_E2E_EFFECTS_ROOT/escape\"\n",
        ),
    )
    .expect("public symlink attack case should be valid")
    .expected(ExpectedEffects::success());
    let symlink_error = runner
        .run(symlink_case)
        .expect_err("public symlink effects must fail closed");
    let symlink_report = symlink_error
        .report()
        .expect("public symlink failure should retain evidence");
    assert!(
        symlink_report
            .containment
            .nonregular_paths
            .iter()
            .any(|path| path.to_string_lossy().contains("escape"))
    );
    fs::remove_dir_all(&symlink_report.root).expect("remove retained symlink evidence");

    let hard_link_case = RunnerCase::new(
        "public-hard-link-attack",
        FixtureBinarySpec::shell(
            "public-hard-link-attack",
            "#!/bin/sh\nln \"$OMNIREPO_E2E_OUTSIDE_CANARY/sentinel\" \"$OMNIREPO_E2E_EFFECTS_ROOT/hard-link\"\n",
        ),
    )
    .expect("public hard-link attack case should be valid")
    .expected(ExpectedEffects::success());
    let hard_link_error = runner
        .run(hard_link_case)
        .expect_err("public hard-link effects must fail closed");
    let hard_link_report = hard_link_error
        .report()
        .expect("public hard-link failure should retain evidence");
    assert!(
        hard_link_report
            .containment
            .hard_link_paths
            .iter()
            .any(|path| path.to_string_lossy().contains("hard-link"))
    );
    fs::remove_dir_all(&hard_link_report.root).expect("remove retained hard-link evidence");

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

fn identity(case_id: &str, seed: u64) -> TestIdentity {
    TestIdentity::new(
        case_id,
        "isolation-stress",
        "fixture-suite",
        "compose",
        SourcePlanConfig::new("source-v1", "plan-v1", "config-v1")
            .expect("source/plan/config identity should be valid"),
        1,
        seed,
        "component",
    )
    .expect("test identity should be valid")
}

fn artifact(case_id: &str) -> ArtifactReference {
    ArtifactReference::new(
        PathBuf::from(format!("cases/{case_id}.jsonl")),
        format!("replay-{case_id}"),
    )
    .expect("artifact reference should be safe")
}

fn record_structured_capability_skip(
    fixture: &mut LifecycleFixture,
    case_id: &str,
    seed: u64,
    unsupported: UnsupportedCapability,
) {
    let recorder = EventRecorder::new(DiagnosticRedactor::default());
    let identity = identity(case_id, seed);
    let artifact = ArtifactReference::new(
        PathBuf::from(format!("capability-skips/{case_id}.jsonl")),
        format!("skip-{case_id}"),
    )
    .expect("skip evidence artifact should be valid");
    let step = recorder
        .start(identity, artifact.clone())
        .expect("capability skip should start an evidence event");
    let diagnostic = format!("capability skipped: {unsupported}");
    step.skip(&diagnostic)
        .expect("capability skip should terminalize as skipped");
    let bundle = recorder
        .finalize()
        .expect("capability skip should finalize evidence");
    assert_eq!(bundle.projection.outcome, Outcome::Skipped);
    assert_eq!(bundle.projection.passed, 0);
    assert_eq!(bundle.projection.skipped, 1);
    assert!(bundle.events.iter().any(|event| {
        event.event_kind == EventKind::Terminal
            && event.outcome == Outcome::Skipped
            && event.diagnostic.as_deref() == Some(diagnostic.as_str())
    }));

    let store = ArtifactStore::new(fixture.roots().artifacts())
        .expect("fixture artifact store should be available");
    let persisted = store
        .write_bundle(
            artifact.path().expect("skip artifact should have a path"),
            &bundle,
        )
        .expect("capability skip evidence should persist");
    let persisted_bundle = omnirepo_test_support::test_evidence::EvidenceBundle::from_jsonl(
        &fs::read_to_string(
            store
                .resolve(
                    persisted
                        .path()
                        .expect("persisted artifact should have a path"),
                )
                .expect("persisted artifact should remain contained"),
        )
        .expect("persisted skip evidence should be readable"),
    )
    .expect("persisted skip evidence should validate");
    assert_eq!(persisted_bundle.projection.outcome, Outcome::Skipped);
    assert_eq!(persisted_bundle.projection.skipped, 1);
    assert!(persisted_bundle.events.iter().any(|event| {
        event.event_kind == EventKind::Terminal
            && event.outcome == Outcome::Skipped
            && event.diagnostic.as_deref() == Some(diagnostic.as_str())
    }));
}

fn require_or_record_capability_skip(
    fixture: &mut LifecycleFixture,
    case_id: &str,
    seed: u64,
    capabilities: &[Capability],
) -> bool {
    for capability in capabilities {
        match fixture.require(*capability) {
            Ok(()) => {}
            Err(FixtureError::Unsupported(unsupported)) => {
                record_structured_capability_skip(fixture, case_id, seed, unsupported);
                return false;
            }
            Err(error) => panic!("capability probe failed: {error}"),
        }
    }
    true
}

fn aggregate_in_order(order: &[usize]) -> String {
    let recorder = EventRecorder::new(DiagnosticRedactor::new(["fixture-secret"]));
    for index in 0..order.len() {
        recorder
            .expect(identity(&format!("case-{index}"), 92_000 + index as u64))
            .expect("peer registration should be unique");
    }
    for index in order {
        let case_id = format!("case-{index}");
        let mut guard = recorder
            .start(
                identity(&case_id, 92_000 + *index as u64),
                artifact(&case_id),
            )
            .expect("worker start should be accepted");
        let outcome = match index {
            1 => Outcome::Failed,
            2 => Outcome::Skipped,
            _ => Outcome::Passed,
        };
        guard
            .finish_with_duration(outcome, (*index as u64) + 3, Some("fixture-secret"))
            .expect("worker should terminalize deterministically");
    }
    recorder
        .finalize()
        .expect("all peers should finalize")
        .to_jsonl()
        .expect("evidence should serialize")
}

fn deterministic_permutation(seed: u64, count: usize) -> Vec<usize> {
    let mut order = (0..count).collect::<Vec<_>>();
    let mut state = seed;
    for index in (1..count).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let swap = (state as usize) % (index + 1);
        order.swap(index, swap);
    }
    order
}

#[test]
fn random_case_order_has_equivalent_sorted_evidence_and_peer_accounting() {
    let first_order = deterministic_permutation(92_101, 6);
    let same_seed_order = deterministic_permutation(92_101, 6);
    let second_order = deterministic_permutation(92_102, 6);
    assert_ne!(first_order, (0..6).collect::<Vec<_>>());
    assert_eq!(first_order, same_seed_order);
    assert_ne!(first_order, second_order);
    let first = aggregate_in_order(&first_order);
    let second = aggregate_in_order(&second_order);
    assert_eq!(
        first, second,
        "completion order must not alter evidence bytes"
    );
    let bundle = omnirepo_test_support::test_evidence::EvidenceBundle::from_jsonl(&first)
        .expect("deterministic evidence should round-trip");
    assert_eq!(bundle.peer_accounting.expected_case_ids.len(), 6);
    assert!(bundle.peer_accounting.missing_case_ids.is_empty());
    assert_eq!(bundle.projection.outcome, Outcome::Failed);
    assert_eq!(bundle.projection.failed, 1);
    assert_eq!(bundle.projection.skipped, 1);
    assert!(first.contains("[REDACTED]"));
}

#[derive(Debug)]
struct ParallelWorkerEvidence {
    root: PathBuf,
    home: PathBuf,
    record: PathBuf,
    run_id: String,
    clock: u64,
    endpoint: String,
    git_root: Option<PathBuf>,
    git_porcelain: Option<String>,
    process_home: String,
    credentials_absent: bool,
    cleanup: omnirepo_test_support::lifecycle_fixture::CleanupReport,
}

fn run_parallel_worker(index: usize, start: Arc<Barrier>) -> ParallelWorkerEvidence {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new(
        format!("parallel-isolation-{index}"),
        93_000 + index as u64,
    ))
    .expect("parallel fixture should be created");
    let root = fixture.roots().root().to_path_buf();
    let home = fixture.roots().home().to_path_buf();
    let record = fixture.roots().runs().join("worker.record");
    fs::write(&record, format!("case={index}\n")).expect("worker record should be written");
    fixture
        .track_ephemeral(&record)
        .expect("worker record should be fixture-contained");
    let clock = fixture.clock();
    clock.advance(Duration::from_millis(100 + index as u64));
    let run_id = fixture.identities().run_id();

    let git_snapshot = match fixture.require(Capability::Git) {
        Ok(()) => Some(
            fixture
                .create_git_repository(
                    fixture.roots().artifacts().join("worker-git"),
                    DirtyGitState::Clean,
                )
                .expect("fixture Git repository should be isolated"),
        ),
        Err(FixtureError::Unsupported(_)) => None,
        Err(error) => panic!("capability probe failed: {error}"),
    };

    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new(
            format!("parallel-process-{index}"),
            ProcessBehavior::Barrier,
        ),
    )
    .expect("fixture process should spawn");
    process
        .wait_for_barrier()
        .expect("fixture process should reach its named barrier");

    let mut server = LocalHttpDouble::bind(
        &mut fixture,
        &format!("parallel-{index}"),
        HttpDoubleSpec::new(200, b"ok\n".to_vec()),
    )
    .expect("local port double should bind");
    let endpoint = server.endpoint().to_owned();
    let endpoint_for_client = endpoint.clone();
    let client = thread::spawn(move || {
        LocalHttpDouble::send_request(
            &endpoint_for_client,
            b"GET /isolated HTTP/1.1\r\nHost: fixture\r\n\r\n",
        )
    });

    start.wait();
    let request = server
        .wait_for_request()
        .expect("local request should reach its barrier");
    assert_eq!(request.path, "/isolated");
    server.respond().expect("local response should release");
    let response = client
        .join()
        .expect("local client should join")
        .expect("local request should complete");
    assert!(String::from_utf8_lossy(&response).contains("200 OK"));
    let exchange = server.finish().expect("local server should join");
    assert_eq!(exchange.response.status, 200);

    process.release().expect("process barrier should release");
    let process_result = process.wait().expect("process tree should be reaped");
    assert!(process_result.status.success());
    assert!(process_result.evidence.ssh_auth_sock_absent);
    assert!(process_result.evidence.ambient_credentials_absent);

    let cleanup = fixture.cleanup(FixtureOutcome::Success);
    ParallelWorkerEvidence {
        root,
        home,
        record,
        run_id,
        clock: clock.now(),
        endpoint,
        git_root: git_snapshot.as_ref().map(|snapshot| snapshot.root.clone()),
        git_porcelain: git_snapshot.map(|snapshot| snapshot.porcelain),
        process_home: process_result.evidence.home,
        credentials_absent: process_result.evidence.ambient_credentials_absent,
        cleanup,
    }
}

#[test]
fn parallel_workers_have_disjoint_home_ports_records_clocks_credentials_and_git() {
    let start = Arc::new(Barrier::new(4));
    let workers = (0..4)
        .map(|index| {
            let start = Arc::clone(&start);
            thread::spawn(move || run_parallel_worker(index, start))
        })
        .collect::<Vec<_>>();
    let workers = workers
        .into_iter()
        .map(|worker| worker.join().expect("parallel worker should join"))
        .collect::<Vec<_>>();

    let roots = workers
        .iter()
        .map(|worker| worker.root.clone())
        .collect::<BTreeSet<_>>();
    let homes = workers
        .iter()
        .map(|worker| worker.home.clone())
        .collect::<BTreeSet<_>>();
    let records = workers
        .iter()
        .map(|worker| worker.record.clone())
        .collect::<BTreeSet<_>>();
    let endpoints = workers
        .iter()
        .map(|worker| worker.endpoint.clone())
        .collect::<BTreeSet<_>>();
    let run_ids = workers
        .iter()
        .map(|worker| worker.run_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(roots.len(), workers.len());
    assert_eq!(homes.len(), workers.len());
    assert_eq!(records.len(), workers.len());
    assert_eq!(endpoints.len(), workers.len());
    assert_eq!(run_ids.len(), workers.len());
    assert!(
        workers
            .iter()
            .all(|worker| worker.record.starts_with(&worker.root))
    );
    assert!(
        workers
            .iter()
            .all(|worker| worker.home.starts_with(&worker.root))
    );
    assert!(
        workers
            .iter()
            .all(|worker| worker.process_home == worker.home.display().to_string())
    );
    assert!(workers.iter().all(|worker| worker.credentials_absent));
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker.clock)
            .collect::<Vec<_>>(),
        vec![100, 101, 102, 103]
    );
    let git_workers = workers
        .iter()
        .filter_map(|worker| worker.git_root.as_ref())
        .collect::<Vec<_>>();
    if !git_workers.is_empty() {
        assert_eq!(git_workers.len(), workers.len());
        let git_roots = git_workers.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(git_roots.len(), workers.len());
        assert_eq!(
            workers
                .iter()
                .filter_map(|worker| worker.git_porcelain.as_deref())
                .collect::<Vec<_>>(),
            vec!["", "", "", ""]
        );
    }
    for worker in workers {
        assert!(worker.cleanup.removed);
        assert!(worker.cleanup.leaks.is_empty());
        assert!(!worker.root.exists());
    }
}

fn parallel_aggregate_evidence() -> String {
    let recorder = EventRecorder::new(DiagnosticRedactor::new(["worker-secret"]));
    let cases = (0..4)
        .map(|index| {
            (
                identity(&format!("parallel-case-{index}"), 94_000 + index as u64),
                artifact(&format!("parallel-case-{index}")),
            )
        })
        .collect::<Vec<_>>();
    for (identity, _) in &cases {
        recorder
            .expect(identity.clone())
            .expect("parallel peer should register once");
    }
    let start = Arc::new(Barrier::new(cases.len()));
    let workers = cases
        .into_iter()
        .enumerate()
        .map(|(index, (identity, artifact))| {
            let recorder = recorder.clone();
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                let mut guard = recorder
                    .start(identity, artifact)
                    .expect("parallel worker should start");
                let outcome = if index == 0 {
                    Outcome::Skipped
                } else {
                    Outcome::Passed
                };
                guard
                    .finish_with_duration(outcome, 19 + index as u64, Some("worker-secret"))
                    .expect("parallel worker should finish");
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("parallel evidence worker should join");
    }
    recorder
        .finalize()
        .expect("parallel evidence should finalize")
        .to_jsonl()
        .expect("parallel evidence should serialize")
}

#[test]
fn parallel_completion_order_is_sorted_and_all_peers_remain_accounted() {
    let first = parallel_aggregate_evidence();
    let second = parallel_aggregate_evidence();
    assert_eq!(
        first, second,
        "parallel completion must be replay-equivalent"
    );
    let bundle = omnirepo_test_support::test_evidence::EvidenceBundle::from_jsonl(&first)
        .expect("parallel bundle should round-trip");
    assert!(bundle.peer_accounting.missing_case_ids.is_empty());
    assert_eq!(bundle.projection.outcome, Outcome::Passed);
    assert_eq!(bundle.projection.passed, 3);
    assert_eq!(bundle.projection.skipped, 1);
    assert!(
        bundle
            .events
            .iter()
            .filter_map(|event| event.diagnostic.as_deref())
            .all(|diagnostic| diagnostic == "[REDACTED]")
    );
}

#[test]
fn cleanup_failure_is_a_harness_failure_without_hiding_the_body_failure() {
    let recorder = EventRecorder::default();
    let execution = execute_case(
        &recorder,
        identity("cleanup-preserves-body", 95_001),
        artifact("cleanup-preserves-body"),
        || Err("original body assertion".to_owned()),
        || Err("cleanup could not remove residue".to_owned()),
    )
    .expect("case execution should retain both failure channels");
    assert_eq!(execution.outcome, Outcome::HarnessFailure);
    assert_eq!(
        execution.body_diagnostic.as_deref(),
        Some("original body assertion")
    );
    assert_eq!(
        execution.cleanup_diagnostic.as_deref(),
        Some("cleanup could not remove residue")
    );
    let bundle = recorder
        .finalize()
        .expect("cleanup result should finalize into evidence");
    assert_eq!(bundle.projection.outcome, Outcome::HarnessFailure);
    assert_eq!(bundle.projection.harness_failures, 1);
    let diagnostic = bundle.events[1]
        .diagnostic
        .as_deref()
        .expect("terminal event should preserve a combined diagnostic");
    assert!(diagnostic.contains("original body assertion"));
    assert!(diagnostic.contains("cleanup could not remove residue"));
}

#[test]
fn late_writer_and_unexpected_artifact_are_retained_as_distinct_cleanup_evidence() {
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new("late-writer-cleanup", 95_002).retain_always())
            .expect("cleanup fixture should be created");
    let leak = fixture.roots().artifacts().join("unexpected.tmp");
    fs::write(&leak, b"unexpected residue\n").expect("unexpected residue should be written");
    fixture
        .track_ephemeral(&leak)
        .expect("unexpected residue should be fixture-contained");

    let race =
        CleanupRaceControl::start(&mut fixture, "late-writer").expect("late writer should start");
    race.wait_for_writer()
        .expect("late writer should reach its barrier");
    let observation = race
        .cleanup_before_writer()
        .expect("cleanup observation should be deterministic");
    assert!(!observation.existed_before_cleanup);
    assert!(!observation.removed_before_writer);
    race.release_writer()
        .expect("late writer should be released");
    let late = race.join().expect("late writer should be joined");
    assert!(late.writer_started);
    assert!(late.writer_completed);
    assert_eq!(
        fs::read(&late.path).expect("late artifact should exist"),
        b"late-writer\n"
    );
    fixture
        .expect_residue(late.path.clone())
        .expect("late artifact should be declared diagnostic residue");

    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    assert_eq!(report.expected_residue, vec![late.path]);
    assert_eq!(report.leaks, vec![leak]);
    assert!(report.root.exists());
    fs::remove_dir_all(report.root).expect("retained cleanup evidence should be removable");
}

#[test]
fn crash_restart_and_retained_records_use_only_fixture_owned_state() {
    let mut fixture = LifecycleFixture::create(
        FixtureSpec::new("crash-restart-composed", 95_003).retain_always(),
    )
    .expect("crash fixture should be created");
    if !require_or_record_capability_skip(
        &mut fixture,
        "crash-restart-composed",
        95_003,
        &[Capability::UnixPermissions],
    ) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }
    let mut parent = CrashableParent::spawn(
        &mut fixture,
        CrashSpec::at("journal.after-flush")
            .run_id("composed-crash-run")
            .with_state("stage", "journal-flushed")
            .with_state("terminal", "false"),
    )
    .expect("crashable parent should spawn");
    parent
        .wait_for_boundary()
        .expect("crash parent should reach its named boundary");
    let crash = parent.wait().expect("crash parent should be reaped");
    assert_eq!(crash.status.code, Some(137));
    assert!(crash.journal_path.starts_with(fixture.roots().runs()));

    let retained = RetainedState::restart(&fixture, "composed-crash-run")
        .expect("restart should read the fixture journal");
    assert_eq!(retained.boundary, "journal.after-flush");
    assert_eq!(retained.field("stage"), Some("journal-flushed"));
    assert_eq!(retained.field("terminal"), Some("false"));
    fixture
        .expect_residue(crash.journal_path.clone())
        .expect("crash journal should be explicitly retained");
    for entry in fs::read_dir(fixture.roots().artifacts()).unwrap() {
        fixture
            .expect_residue(entry.unwrap().path())
            .expect("crash helper artifacts should have explicit ownership");
    }
    let report = fixture.cleanup(FixtureOutcome::Failure);
    assert!(report.retained);
    assert!(report.leaks.is_empty());
    fs::remove_dir_all(report.root).expect("retained crash evidence should be removable");
}

#[test]
fn colliding_identity_is_rejected_before_parallel_run_effects() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("collision-composed", 95_004))
        .expect("collision fixture should be created");
    if !require_or_record_capability_skip(
        &mut fixture,
        "collision-composed",
        95_004,
        &[Capability::UnixPermissions],
    ) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }
    let identities = fixture.identities();
    identities.force_collision("run", "collision-run");
    let run_ids = [identities.run_id(), identities.run_id()];
    assert_eq!(run_ids[0], run_ids[1]);
    let error = ConcurrentRunControl::launch(&mut fixture, run_ids.into_iter().collect::<Vec<_>>())
        .err()
        .expect("colliding run IDs must fail closed");
    assert!(error.to_string().contains("duplicate concurrent run"));
    assert_eq!(fs::read_dir(fixture.roots().runs()).unwrap().count(), 0);
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn hostile_name_in_a_late_parallel_case_reaps_prior_workers_and_leaves_no_records() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("hostile-name-composed", 95_005))
        .expect("hostile-name fixture should be created");
    if !require_or_record_capability_skip(
        &mut fixture,
        "hostile-name-composed",
        95_005,
        &[Capability::UnixPermissions],
    ) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }
    let error = ConcurrentRunControl::launch(
        &mut fixture,
        vec!["valid-worker".to_owned(), "../hostile-worker".to_owned()],
    )
    .err()
    .expect("hostile run name must fail before effects escape");
    assert!(error.to_string().contains("invalid run_id"));
    assert_eq!(fs::read_dir(fixture.roots().runs()).unwrap().count(), 0);
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn interrupted_artifact_write_is_visible_and_recoverable() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("interrupted-write", 95_006))
        .expect("writer fixture should be created");
    let mut writer = JournalWriterControl::create(
        &mut fixture,
        "interrupted",
        Some(WriterFault::EnospcAfter(5)),
    )
    .expect("writer control should create a fixture path");
    let error = writer
        .write(b"complete-artifact")
        .expect_err("injected interruption must remain visible");
    assert_eq!(
        error,
        WriterError::Enospc {
            attempted: 17,
            written: 5
        }
    );
    assert_eq!(fs::read(writer.path()).unwrap(), b"compl");
    let recovered = writer
        .write(b"complete")
        .expect("one-shot fault should not poison later recovery");
    assert_eq!(recovered.written, 8);
    assert_eq!(fs::read(writer.path()).unwrap(), b"complete");
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn unsupported_capability_is_a_terminal_skip_and_never_a_pass() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("capability-skip", 95_007))
        .expect("capability fixture should be created");
    match fixture.require(Capability::Fifo) {
        Ok(()) => {
            let recorder = EventRecorder::default();
            let mut step = recorder
                .start(
                    identity("capability-skip", 95_007),
                    artifact("capability-skip"),
                )
                .expect("capability case should start");
            step.finish_with_duration(Outcome::Passed, 0, Some("fifo capability supported"))
                .expect("supported capability should terminalize as passed");
            let bundle = recorder
                .finalize()
                .expect("capability result should produce evidence");
            assert_eq!(bundle.projection.outcome, Outcome::Passed);
            assert_eq!(bundle.projection.skipped, 0);
        }
        Err(FixtureError::Unsupported(unsupported)) => {
            record_structured_capability_skip(&mut fixture, "capability-skip", 95_007, unsupported);
        }
        Err(error) => panic!("capability probe failed: {error}"),
    }
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn capability_skip_evidence_is_persisted_without_stdout() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("forced-capability-skip", 95_008))
        .expect("forced capability fixture should be created");
    record_structured_capability_skip(
        &mut fixture,
        "forced-capability-skip",
        95_008,
        UnsupportedCapability {
            capability: Capability::Symlink,
            reason: "forced test capability unavailable".to_owned(),
        },
    );
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

fn e2e_isolation_case(index: usize) -> RunnerCase {
    let case_id = format!("isolation-e2e-{index}");
    let script = r##"#!/bin/sh
set -eu
test "$HOME" = "$OMNIREPO_E2E_HOME"
test "$USERPROFILE" = "$OMNIREPO_E2E_HOME"
test "$GIT_CONFIG_NOSYSTEM" = "1"
test "$GIT_CONFIG_GLOBAL" = "$OMNIREPO_E2E_ROOT/gitconfig.global"
test "$GIT_CONFIG_SYSTEM" = "$OMNIREPO_E2E_ROOT/gitconfig.system"
test "$OMNIREPO_E2E_OFFLINE" = "1"
test "$OMNIREPO_E2E_NO_AMBIENT_CREDENTIALS" = "1"
test -z "${SSH_AUTH_SOCK-}"
test -z "${AWS_ACCESS_KEY_ID-}"
test -z "${AWS_SECRET_ACCESS_KEY-}"
test -z "${GITHUB_TOKEN-}"
printf 'case=%s\n' "$OMNIREPO_E2E_CASE_ID" > "$OMNIREPO_E2E_EFFECTS_ROOT/case.txt"
"##;
    RunnerCase::new(
        &case_id,
        FixtureBinarySpec::shell(format!("isolation-e2e-{index}"), script),
    )
    .expect("E2E isolation case should be valid")
    .seed(96_000 + index as u64)
    .expected(
        ExpectedEffects::success().exact_files([ExpectedFile::with_contents(
            "case.txt",
            format!("case={case_id}\n"),
        )]),
    )
}

#[test]
fn composed_e2e_cases_are_offline_fixture_only_and_leave_no_real_state() {
    let mut capability_fixture =
        LifecycleFixture::create(FixtureSpec::new("composed-e2e-capability-probe", 96_101))
            .expect("E2E capability fixture should be created");
    if !require_or_record_capability_skip(
        &mut capability_fixture,
        "composed-e2e-capability-probe",
        96_101,
        &[Capability::UnixPermissions, Capability::Git],
    ) {
        let report = capability_fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }
    let report = capability_fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    let order = deterministic_permutation(96_101, 3);
    let runner = E2eRunner::new();
    let mut replay_ids = BTreeSet::new();
    for index in order {
        let report = runner
            .run(e2e_isolation_case(index))
            .expect("clean E2E isolation case should pass");
        assert_eq!(report.process.code, Some(0));
        assert!(report.process.signal.is_none());
        assert!(report.process.stdout.bytes.is_empty());
        assert!(report.process.stderr.bytes.is_empty());
        assert!(report.containment.no_outside_writes());
        assert!(!report.git.unexpected_changes);
        assert!(report.git.source_before == report.git.source_after);
        assert!(report.git.destination_before == report.git.destination_after);
        assert!(report.git.remote_before == report.git.remote_after);
        assert_eq!(report.evidence_bundle.projection.outcome, Outcome::Passed);
        assert!(report.cleanup.removed);
        assert!(!report.root.exists());
        assert!(replay_ids.insert(report.replay_id));
    }
    assert_eq!(replay_ids.len(), 3);
}

#[test]
fn descendant_late_write_and_bounded_output_are_reaped_before_fixture_cleanup() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("descendant-stress", 96_102))
        .expect("descendant fixture should be created");
    if !require_or_record_capability_skip(
        &mut fixture,
        "descendant-stress",
        96_102,
        &[Capability::UnixPermissions],
    ) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
        return;
    }
    let mut descendant = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("descendant", ProcessBehavior::ForkAndLateWrite),
    )
    .expect("descendant process should spawn");
    descendant
        .wait_for_barrier()
        .expect("descendant parent should reach its barrier");
    descendant.release().expect("descendant should release");
    let descendant_result = descendant.wait().expect("descendant tree should be reaped");
    assert!(descendant_result.status.success());
    assert!(descendant_result.evidence.late_write);
    assert!(descendant_result.evidence.ambient_credentials_absent);

    let mut output = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new(
            "bounded-output",
            ProcessBehavior::OversizedStdoutAndStderr {
                stdout_chunks: vec!["stdout-".repeat(20)],
                stderr_chunks: vec!["stderr-".repeat(20)],
            },
        )
        .with_output_limit(32),
    )
    .expect("bounded output process should spawn");
    output
        .wait_for_barrier()
        .expect("bounded output process should reach its barrier");
    output
        .release()
        .expect("bounded output process should release");
    let output_result = output
        .wait()
        .expect("bounded output process should be reaped");
    assert!(output_result.stdout.truncated);
    assert!(output_result.stderr.truncated);
    assert_eq!(output_result.stdout.bytes.len(), 32);
    assert_eq!(output_result.stderr.bytes.len(), 32);

    let mut hanging = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("drop-hanging", ProcessBehavior::Hang),
    )
    .expect("hanging process should spawn");
    hanging
        .wait_for_barrier()
        .expect("hanging process should reach its barrier");
    drop(hanging);
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn missing_worker_is_synthesized_as_harness_failure_with_peer_accounting() {
    let recorder = EventRecorder::default();
    let missing = identity("missing-worker", 96_103);
    let completed = identity("completed-worker", 96_104);
    recorder
        .expect(missing)
        .expect("missing worker should be registered before dispatch");
    recorder
        .expect(completed.clone())
        .expect("completed worker should be registered before dispatch");
    let mut step = recorder
        .start(completed, artifact("completed-worker"))
        .expect("completed worker should start");
    step.finish_with_duration(Outcome::Passed, 4, None)
        .expect("completed worker should finish");
    let bundle = recorder
        .finalize()
        .expect("missing worker should be synthesized at finalization");
    assert_eq!(bundle.projection.outcome, Outcome::HarnessFailure);
    assert_eq!(bundle.projection.missing, 0);
    assert_eq!(bundle.projection.harness_failures, 1);
    assert_eq!(bundle.projection.passed, 1);
    assert!(bundle.peer_accounting.missing_case_ids.is_empty());
    assert_eq!(bundle.peer_accounting.terminal_case_ids.len(), 2);
    assert!(
        bundle
            .peer_accounting
            .terminal_outcomes
            .values()
            .any(|outcome| *outcome == Outcome::HarnessFailure)
    );
}
