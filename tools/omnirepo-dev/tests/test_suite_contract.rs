//! Contract tests for the repository-owned feature-test orchestrator.
//!
//! The fixtures below are deliberately tiny.  They exercise the process
//! boundary and report contract without running the repository's real quality
//! gates or the canonical journey target owned by `.74.7`.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use omnirepo_dev::test_suite::{
    CancellationToken, CaseOutcome, RunnerError, RunnerOptions, Selection, SuiteKind,
    TEST_SUITE_EVENT_SCHEMA, TEST_SUITE_MANIFEST_SCHEMA, TEST_SUITE_REPORT_SCHEMA, run,
};
use serde_json::{Value, json};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

fn root(name: &str) -> PathBuf {
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    // The system temp dir on macOS lives under /var/folders, and /var is a
    // symlink there; the test-suite runner validates the artifact root as
    // symlink-free, so fixtures live under the repository.
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let root = base.join(format!(
        "omnirepo-test-suite-{name}-{}-{sequence}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create test-suite fixture root");
    root
}

fn write_manifest(root: &Path, suites: Value) -> PathBuf {
    let path = root.join("test-suite.json");
    let manifest = json!({
        "schema": TEST_SUITE_MANIFEST_SCHEMA,
        "version": 1,
        "suites": suites,
    });
    fs::write(
        &path,
        serde_json::to_vec(&manifest).expect("serialize test-suite manifest"),
    )
    .expect("write test-suite manifest");
    path
}

fn case(id: &str, argv: &[&str]) -> Value {
    json!({
        "id": id,
        "argv": argv,
        "working_directory": ".",
        "seed": 7,
    })
}

fn suite(id: &str, kind: &str, cases: Vec<Value>) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "cases": cases,
    })
}

fn options(manifest: &Path, root: &Path) -> RunnerOptions {
    RunnerOptions::new(manifest, root).with_artifacts(root.join("artifacts"))
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

#[test]
fn full_selection_is_ordered_and_preserves_the_first_failure_status() {
    let fixture = root("failure-status");
    let manifest = write_manifest(
        &fixture,
        json!([
            suite(
                "unit",
                "unit",
                vec![
                    case(
                        "first-failure",
                        &[
                            "/bin/sh",
                            "-c",
                            "printf worker-out; printf worker-err >&2; exit 23"
                        ]
                    ),
                    case("later-pass", &["/bin/sh", "-c", "printf later; exit 0"]),
                ],
            ),
            suite(
                "component",
                "component",
                vec![case("last-failure", &["/bin/sh", "-c", "exit 17"])],
            ),
        ]),
    );

    let report = run(&options(&manifest, &fixture).with_jobs(3)).expect("run full suite");
    assert_eq!(report.schema, TEST_SUITE_REPORT_SCHEMA);
    assert!(!report.success);
    assert_eq!(report.exit_code, 23);
    assert_eq!(
        report
            .cases
            .iter()
            .map(|case| case.id.as_str())
            .collect::<Vec<_>>(),
        ["first-failure", "later-pass", "last-failure"]
    );
    assert_eq!(report.cases[0].outcome, CaseOutcome::Failed);
    assert_eq!(report.cases[0].exit_code, Some(23));
    assert_eq!(report.cases[1].outcome, CaseOutcome::Passed);
    assert_eq!(report.cases[2].exit_code, Some(17));
    assert!(
        report.cases[0]
            .stdout
            .ends_with("/cases/unit/first-failure/stdout.log")
    );
    assert!(
        report.cases[0]
            .stderr
            .ends_with("/cases/unit/first-failure/stderr.log")
    );
    let event_path = fixture.join("artifacts").join(&report.event_log);
    let event_log = fs::read_to_string(event_path).expect("event log is retained");
    assert!(event_log.contains(TEST_SUITE_EVENT_SCHEMA));
    assert!(event_log.contains("first-failure"));
    assert!(
        fixture
            .join("artifacts")
            .join(&report.cases[0].stdout)
            .is_file()
    );
    assert!(
        fixture
            .join("artifacts")
            .join(&report.cases[0].stderr)
            .is_file()
    );
    let replay = report.cases[0].replay.as_ref().expect("replay reference");
    assert!(fixture.join("artifacts").join(&replay.artifact).is_file());
    cleanup(&fixture);
}

#[cfg(unix)]
#[test]
fn signal_failure_retains_a_nonzero_suite_status() {
    let fixture = root("signal-status");
    let manifest = write_manifest(
        &fixture,
        json!([suite(
            "unit",
            "unit",
            vec![case("signal-failure", &["/bin/sh", "-c", "kill -TERM $$"])],
        )]),
    );

    let report = run(&options(&manifest, &fixture)).expect("signal failure is reportable");
    assert!(!report.success);
    assert_eq!(report.cases[0].outcome, CaseOutcome::Failed);
    assert_eq!(report.cases[0].signal, Some(15));
    assert_eq!(report.cases[0].exit_code, None);
    assert_eq!(report.exit_code, 143);
    cleanup(&fixture);
}

#[test]
fn case_and_suite_selection_are_exclusive_and_deterministic() {
    let fixture = root("selection");
    let manifest = write_manifest(
        &fixture,
        json!([
            suite(
                "unit",
                "unit",
                vec![
                    case("unit-one", &["/bin/sh", "-c", "exit 0"]),
                    case("unit-two", &["/bin/sh", "-c", "exit 0"]),
                ],
            ),
            suite(
                "e2e",
                "e2e",
                vec![case("journey-one", &["/bin/sh", "-c", "exit 0"])],
            ),
        ]),
    );

    let one = run(&options(&manifest, &fixture)
        .with_selection(Selection::Case("unit-two".to_owned()))
        .with_jobs(2))
    .expect("case selection runs");
    assert_eq!(one.cases.len(), 1);
    assert_eq!(one.cases[0].id, "unit-two");

    let e2e = run(&options(&manifest, &fixture)
        .with_selection(Selection::Suite("e2e".to_owned()))
        .with_jobs(2))
    .expect("suite selection runs");
    assert_eq!(e2e.cases.len(), 1);
    assert_eq!(e2e.cases[0].suite, "e2e");
    assert_eq!(e2e.suites, vec!["e2e"]);

    assert!(matches!(
        Selection::parse(Some("unit-one"), Some("e2e"), false),
        Err(RunnerError::InvalidSelection { .. })
    ));
    cleanup(&fixture);
}

#[test]
fn unsupported_capability_and_missing_tool_are_typed_failures() {
    let fixture = root("typed-failures");
    let path = fixture.join("test-suite.json");
    let manifest = json!({
        "schema": TEST_SUITE_MANIFEST_SCHEMA,
        "version": 1,
        "suites": [{
            "id": "platform",
            "kind": "platform",
            "cases": [
                {
                    "id": "unsupported-macos",
                    "argv": ["/bin/sh", "-c", "exit 0"],
                    "capabilities": [{"name": "macos-apfs", "supported": false, "detail": "linux fixture"}]
                },
                {
                    "id": "missing-tool",
                    "argv": ["/definitely/missing/omnirepo-tool"]
                }
            ]
        }]
    });
    fs::write(
        &path,
        serde_json::to_vec(&manifest).expect("serialize typed fixture"),
    )
    .expect("write typed fixture");

    let report = run(&options(&path, &fixture)).expect("typed failures remain reportable");
    assert!(!report.success);
    assert_eq!(report.cases[0].outcome, CaseOutcome::UnsupportedCapability);
    assert_eq!(report.cases[0].exit_code, Some(125));
    assert_eq!(report.cases[1].outcome, CaseOutcome::MissingTool);
    assert_eq!(report.cases[1].exit_code, Some(127));
    assert!(report.cases[0].replay.is_some());
    assert!(report.cases[1].replay.is_some());
    cleanup(&fixture);
}

#[test]
fn host_bound_capability_mismatch_is_a_visible_skip() {
    let fixture = root("host-skip");
    let path = fixture.join("test-suite.json");
    let manifest = json!({
        "schema": TEST_SUITE_MANIFEST_SCHEMA,
        "version": 1,
        "suites": [{
            "id": "platform",
            "kind": "platform",
            "cases": [{
                "id": "host-bound",
                "argv": ["/bin/sh", "-c", "echo should-not-run"],
                "capabilities": [{"name": "linux-filesystem", "supported": true}]
            }]
        }]
    });
    fs::write(
        &path,
        serde_json::to_vec(&manifest).expect("serialize host fixture"),
    )
    .expect("write host fixture");

    let report = run(&options(&path, &fixture)).expect("host resolution remains reportable");
    // The shared manifest declares the capability; the host resolves it.  On
    // Linux the case runs; on any other host it is a recorded visible skip,
    // and the run stays green either way.
    assert!(report.success, "host resolution must not fail the run");
    if cfg!(target_os = "linux") {
        assert_eq!(report.cases[0].outcome, CaseOutcome::Passed);
    } else {
        assert_eq!(report.cases[0].outcome, CaseOutcome::HostUnsupported);
        assert!(
            report.cases[0]
                .diagnostic
                .as_deref()
                .is_some_and(|diagnostic| diagnostic.contains("host does not support")),
            "the skip reason is visible in the report"
        );
    }
    cleanup(&fixture);
}

#[test]
fn invalid_manifest_and_zero_parallelism_fail_before_spawning() {
    let fixture = root("preflight");
    let marker = fixture.join("started");
    let manifest = write_manifest(
        &fixture,
        json!([suite(
            "unit",
            "unit",
            vec![case(
                "would-start",
                &["/bin/sh", "-c", &format!("touch {}", marker.display())],
            )],
        )]),
    );

    assert!(matches!(
        run(&options(&manifest, &fixture).with_jobs(0)),
        Err(RunnerError::InvalidOptions { .. })
    ));
    assert!(!marker.exists());

    let invalid = fixture.join("invalid.json");
    fs::write(
        &invalid,
        r#"{"schema":"omnirepo.test-suite-manifest.v1","version":1,"suites":[]}"#,
    )
    .expect("write invalid manifest");
    assert!(matches!(
        run(&options(&invalid, &fixture)),
        Err(RunnerError::InvalidManifest { .. })
    ));
    cleanup(&fixture);
}

#[test]
fn cancellation_terminalizes_every_selected_case_without_spawning_workers() {
    let fixture = root("cancellation");
    let marker = fixture.join("started");
    let manifest = write_manifest(
        &fixture,
        json!([suite(
            "unit",
            "unit",
            vec![
                case(
                    "queued-one",
                    &["/bin/sh", "-c", &format!("touch {}", marker.display())],
                ),
                case("queued-two", &["/bin/sh", "-c", "exit 0"]),
            ],
        )]),
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let report = run(&options(&manifest, &fixture)
        .with_cancellation(cancellation)
        .with_jobs(2))
    .expect("cancelled suite still produces a report");
    assert!(!report.success);
    assert_eq!(report.exit_code, 130);
    assert_eq!(
        report
            .cases
            .iter()
            .map(|case| case.outcome)
            .collect::<Vec<_>>(),
        [CaseOutcome::Cancelled, CaseOutcome::Cancelled]
    );
    assert!(!marker.exists());
    cleanup(&fixture);
}

#[test]
fn cli_projects_json_and_status_without_worker_terminal_output() {
    let fixture = root("cli-output");
    let manifest = write_manifest(
        &fixture,
        json!([suite(
            "unit",
            "unit",
            vec![case(
                "cli-failure",
                &[
                    "/bin/sh",
                    "-c",
                    "printf worker-out; printf worker-err >&2; exit 29",
                ],
            )],
        )]),
    );

    let output = omnirepo_dev::run([
        "test",
        "--manifest",
        manifest.to_str().expect("manifest path is UTF-8"),
        "--repo-root",
        fixture.to_str().expect("fixture path is UTF-8"),
        "--case",
        "cli-failure",
        "--artifacts",
        fixture
            .join("artifacts")
            .to_str()
            .expect("artifact path is UTF-8"),
        "--json",
    ]);
    assert_eq!(output.status, 29);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.lines().count(), 1);
    let report: Value = serde_json::from_str(&output.stdout).expect("CLI emits report JSON");
    assert_eq!(report["exit_code"], 29);
    assert_eq!(report["cases"][0]["outcome"], "failed");
    cleanup(&fixture);
}

#[test]
fn suite_kind_wire_values_remain_explicit() {
    let fixture = root("kinds");
    let manifest = write_manifest(
        &fixture,
        json!([
            suite(
                "unit",
                "unit",
                vec![case("u", &["/bin/sh", "-c", "exit 0"])]
            ),
            suite(
                "component",
                "component",
                vec![case("c", &["/bin/sh", "-c", "exit 0"])]
            ),
            suite("e2e", "e2e", vec![case("e", &["/bin/sh", "-c", "exit 0"])]),
            suite(
                "adversarial",
                "adversarial",
                vec![case("a", &["/bin/sh", "-c", "exit 0"])]
            ),
            suite(
                "platform",
                "platform",
                vec![case("p", &["/bin/sh", "-c", "exit 0"])]
            ),
        ]),
    );
    let report = run(&options(&manifest, &fixture)).expect("all suite kinds run");
    assert_eq!(report.cases.len(), 5);
    assert_eq!(SuiteKind::Platform.as_str(), "platform");
    cleanup(&fixture);
}

#[test]
fn quality_status_is_delegated_without_reimplementing_gate_policy() {
    let fixture = root("quality-delegation");
    let manifest = write_manifest(
        &fixture,
        json!([suite(
            "unit",
            "unit",
            vec![case("feature-pass", &["/bin/sh", "-c", "exit 0"])],
        )]),
    );
    let quality_manifest = fixture.join("quality.json");
    let quality = json!({
        "schema": "omnirepo.quality-manifest.v1",
        "version": 1,
        "gates": [{
            "id": "fixture-gate",
            "kind": "gate",
            "toolchain": "fixture",
            "working_directory": ".",
            "argv": ["/bin/sh", "-c", "exit 19"],
            "failure_identity": "quality.fixture"
        }],
        "profiles": [
            {"name": "full", "kind": "profile", "gates": ["fixture-gate"]},
            {"name": "stable", "kind": "profile", "gates": ["fixture-gate"]},
            {"name": "msrv", "kind": "profile", "gates": ["fixture-gate"]},
            {"name": "coverage", "kind": "profile", "gates": ["fixture-gate"]}
        ]
    });
    fs::write(
        &quality_manifest,
        serde_json::to_vec(&quality).expect("serialize quality manifest"),
    )
    .expect("write quality manifest");

    let report = run(&options(&manifest, &fixture)
        .with_quality_manifest("quality.json")
        .with_quality_profile("full"))
    .expect("delegated quality report");
    let quality = report.quality.expect("quality result is retained");
    assert!(quality.delegated);
    assert!(!quality.success);
    assert_eq!(quality.exit_code, 1);
    assert_eq!(quality.failed_gates, vec!["fixture-gate"]);
    assert_eq!(report.exit_code, 1);
    assert!(fixture.join("artifacts").join(quality.artifact).is_file());
    cleanup(&fixture);
}

#[test]
fn concurrent_jobs_never_overlap_the_shared_build_directory() {
    // Every repository case drives cargo against one workspace build
    // directory, so cargo serializes them on a lock the runner cannot
    // see. When the runner overlapped them anyway, a case spent its
    // bounded budget blocked on that lock and was killed having executed
    // nothing (`Blocking waiting for file lock on build directory`).
    // The runner now leases the build directory, so cases never overlap
    // and each one's bound measures its own execution.
    let fixture = root("build-lease");
    let marker = fixture.join("order.log");
    let script = |name: &str| {
        format!(
            "printf '{name}-start\n' >> {0}; sleep 0.5; printf '{name}-end\n' >> {0}",
            marker.display()
        )
    };
    let first = script("first");
    let second = script("second");
    let manifest = write_manifest(
        &fixture,
        json!([suite(
            "unit",
            "unit",
            vec![
                case("first", &["/bin/sh", "-c", first.as_str()]),
                case("second", &["/bin/sh", "-c", second.as_str()]),
            ],
        )]),
    );

    let report = run(&options(&manifest, &fixture).with_jobs(2)).expect("run leased suite");
    assert!(report.success, "both cases must pass: {:?}", report.cases);

    let order = fs::read_to_string(&marker).expect("marker log");
    let lines = order.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        4,
        "each case writes a start and an end: {lines:?}"
    );
    // A case that holds the build directory runs to completion before the
    // next one starts: no start may appear between another case's start
    // and its end.
    assert!(
        lines[0].ends_with("-start")
            && lines[1].ends_with("-end")
            && lines[2].ends_with("-start")
            && lines[3].ends_with("-end"),
        "cases overlapped the build directory: {lines:?}"
    );
    assert_eq!(
        lines[0].trim_end_matches("-start"),
        lines[1].trim_end_matches("-end"),
        "the first case must finish before the second starts: {lines:?}"
    );
    cleanup(&fixture);
}

#[test]
fn a_case_inherits_the_flags_the_workspace_was_built_with() {
    // A case runs with a cleared environment.  `RUSTFLAGS` is part of
    // cargo's fingerprint, so a case that does not inherit it rebuilds a
    // workspace that was warmed under different flags, and charges that
    // rebuild to its own bound: the macOS suite timed out that way with
    // its cases still compiling.
    assert!(
        omnirepo_dev::test_suite::forwarded_environment().contains(&"RUSTFLAGS"),
        "a case must inherit RUSTFLAGS or it cannot reuse the workspace build"
    );
    assert!(
        omnirepo_dev::test_suite::forwarded_environment().contains(&"RUSTDOCFLAGS"),
        "doc flags share the same fingerprint hazard"
    );
    for required in ["PATH", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
        assert!(
            omnirepo_dev::test_suite::forwarded_environment().contains(&required),
            "{required} must reach the case for cargo to run at all"
        );
    }
}
