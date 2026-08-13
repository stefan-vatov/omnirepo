// Usage patterns stay beside the hermetic fixture implementation.

use std::{fs, thread};

use lifecycle_fixture::{
    Capability, FaultAction, FaultPoint, FixtureError, FixtureOutcome, FixtureSpec,
    LifecycleFixture,
};
use process_double::{FakeExecutable, ProcessBehavior, ProcessSpec};
use recovery_control::{ConcurrentRunControl, CrashSpec, CrashableParent, RetainedState};

use omnirepo_test_support::{lifecycle_fixture, process_double, recovery_control};

fn require_unix_permissions(fixture: &LifecycleFixture) -> bool {
    match fixture.require(Capability::UnixPermissions) {
        Ok(()) => true,
        Err(FixtureError::Unsupported(_)) => false,
        Err(error) => panic!("capability probe failed: {error}"),
    }
}

#[test]
fn component_pattern_uses_named_fault_and_barrier_evidence() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("component-pattern", 6101))
        .expect("component fixture should be created");
    if !require_unix_permissions(&fixture) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.leaks.is_empty());
        return;
    }

    let fault = FaultPoint::new("component.read", "before-read", "component-owner", 1);
    fixture
        .faults()
        .arm(
            fault.clone(),
            FaultAction::ReturnError("injected-read-error".to_owned()),
        )
        .expect("fault point should be unique");
    assert_eq!(
        fixture.faults().hit(&fault),
        Some(FaultAction::ReturnError("injected-read-error".to_owned()))
    );
    fixture
        .faults()
        .assert_consumed()
        .expect("named fault should be consumed");

    let barriers = fixture.barriers();
    let barrier = barriers
        .arm("component-ready")
        .expect("component barrier should be unique");
    let worker = thread::spawn(move || barrier.hit());
    barriers
        .wait_for_hit("component-ready")
        .expect("component should reach its barrier");
    barriers
        .release("component-ready")
        .expect("component barrier should release");
    worker
        .join()
        .expect("component worker should join")
        .expect("component worker should observe release");

    let evidence = fixture
        .roots()
        .artifacts()
        .join("component-pattern.evidence");
    fs::write(
        &evidence,
        "owner=component-owner\nfault=component.read:before-read\nbarrier=component-ready\nseed=6101\n",
    )
    .expect("component evidence should be written");
    fixture
        .track_ephemeral(&evidence)
        .expect("component evidence should stay in fixture roots");
    assert!(
        fixture
            .log()
            .snapshot()
            .iter()
            .any(|event| { event.kind == "fixture.create" && event.detail.contains("seed=6101") })
    );

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.leaks.is_empty());
}

#[test]
fn process_tree_pattern_releases_and_reaps_the_fake_executable() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-tree-pattern", 6202))
        .expect("process-tree fixture should be created");
    if !require_unix_permissions(&fixture) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.leaks.is_empty());
        return;
    }

    let mut process = FakeExecutable::spawn(
        &mut fixture,
        ProcessSpec::new("process-tree-pattern", ProcessBehavior::ForkAndLateWrite),
    )
    .expect("fake process should spawn");
    process
        .wait_for_barrier()
        .expect("process should reach the named barrier");
    process.release().expect("process should be released once");
    let result = process.wait().expect("process tree should be reaped");
    assert!(result.status.success());
    assert!(result.evidence.late_write);
    assert!(result.evidence.ssh_auth_sock_absent);
    assert!(result.evidence.ambient_credentials_absent);
    assert!(
        result.evidence.home.starts_with(
            fixture
                .roots()
                .home()
                .to_str()
                .expect("fixture home should be UTF-8")
        )
    );

    let evidence = fixture
        .roots()
        .artifacts()
        .join("process-tree-pattern.evidence");
    fs::write(
        &evidence,
        format!(
            "owner=process-tree\nbarrier={}\nseed=6202\nlate-write={}\n",
            result.evidence.barrier, result.evidence.late_write
        ),
    )
    .expect("process-tree evidence should be written");
    fixture
        .track_ephemeral(&evidence)
        .expect("process-tree evidence should stay in fixture roots");

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.leaks.is_empty());
}

#[test]
fn crash_restart_pattern_replays_retained_state_at_named_boundary() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("crash-restart-pattern", 6303))
        .expect("crash fixture should be created");
    if !require_unix_permissions(&fixture) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.leaks.is_empty());
        return;
    }

    let mut parent = CrashableParent::spawn(
        &mut fixture,
        CrashSpec::at("journal.after-flush")
            .run_id("crash-restart-pattern-run")
            .with_state("fixture_owner", "recovery"),
    )
    .expect("crashable parent should spawn");
    parent
        .wait_for_boundary()
        .expect("parent should report its durable boundary");
    let crash = parent.wait().expect("crashed parent should be reaped");
    assert_eq!(crash.run_id, "crash-restart-pattern-run");
    assert_eq!(crash.boundary, "journal.after-flush");
    assert_eq!(crash.status.code, Some(137));

    let retained = RetainedState::restart(&fixture, "crash-restart-pattern-run")
        .expect("retained state should be replayable");
    assert_eq!(retained.boundary, "journal.after-flush");
    assert_eq!(retained.field("fixture_owner"), Some("recovery"));

    let evidence = fixture
        .roots()
        .artifacts()
        .join("crash-restart-pattern.evidence");
    fs::write(
        &evidence,
        format!(
            "owner=recovery\nfault={}\nseed=6303\njournal={}\n",
            crash.boundary,
            crash.journal_path.display()
        ),
    )
    .expect("crash evidence should be written");
    fixture
        .track_ephemeral(&evidence)
        .expect("crash evidence should stay in fixture roots");

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.leaks.is_empty());
}

#[test]
fn concurrent_fleet_pattern_releases_each_run_and_collects_evidence() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("concurrent-fleet-pattern", 6404))
        .expect("concurrent fixture should be created");
    if !require_unix_permissions(&fixture) {
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.leaks.is_empty());
        return;
    }

    let mut runs = ConcurrentRunControl::launch(
        &mut fixture,
        [
            "fleet-a".to_owned(),
            "fleet-b".to_owned(),
            "fleet-c".to_owned(),
        ],
    )
    .expect("fleet runs should launch");
    runs.wait_for_ready()
        .expect("all fleet runs should hit the ready barrier");
    runs.release_all()
        .expect("all fleet runs should release together");
    let results = runs
        .join()
        .expect("all fleet process trees should be reaped");
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| {
        result.status.code == Some(0) && result.state.get("status") == Some(&"completed".to_owned())
    }));
    assert_eq!(
        results
            .iter()
            .map(|result| result.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["fleet-a", "fleet-b", "fleet-c"]
    );

    let evidence = fixture
        .roots()
        .artifacts()
        .join("concurrent-fleet-pattern.evidence");
    fs::write(
        &evidence,
        "owner=concurrent-fleet\nfault=run.ready\nbarrier=run-ready\nseed=6404\n",
    )
    .expect("fleet evidence should be written");
    fixture
        .track_ephemeral(&evidence)
        .expect("fleet evidence should stay in fixture roots");

    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.leaks.is_empty());
}
