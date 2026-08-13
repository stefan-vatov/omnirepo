//! Deterministic public contracts for the private quality-manifest runner.
//!
//! These tests keep process execution observable without invoking repository
//! gates.  They cover the typed admission failures and the runner's contract:
//! validate the manifest before spawning children, execute selected gates in
//! manifest order, and retain every result after a failure.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use omnirepo_dev::quality::{GateResult, RunnerError, RunnerOptions, run};
use serde_json::{Value, json};

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn root(name: &str) -> PathBuf {
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "omnirepo-quality-edge-{name}-{}-{sequence}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create quality fixture root");
    root
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

fn gate(id: &str, argv: &[String]) -> Value {
    json!({
        "id": id,
        "kind": "gate",
        "toolchain": "fixture-rust",
        "working_directory": ".",
        "argv": argv,
        "failure_identity": format!("quality.fixture-{id}"),
    })
}

fn profile(name: &str, gates: &[&str]) -> Value {
    json!({
        "name": name,
        "kind": "profile",
        "gates": gates,
    })
}

fn manifest(gates: Vec<Value>, profiles: Vec<Value>) -> String {
    serde_json::to_string(&json!({
        "schema": "omnirepo.quality-manifest.v1",
        "version": 1,
        "gates": gates,
        "profiles": profiles,
    }))
    .expect("serialize quality fixture manifest")
}

fn valid_profiles(gates: &[&str]) -> Vec<Value> {
    vec![
        profile("full", gates),
        profile("stable", gates),
        profile("msrv", gates),
        profile("coverage", gates),
    ]
}

fn write_manifest(root: &Path, source: &str) -> PathBuf {
    let path = root.join("quality-manifest.json");
    fs::write(&path, source).expect("write quality fixture manifest");
    path
}

fn options(manifest: &Path, repo_root: &Path) -> RunnerOptions {
    RunnerOptions::new(manifest, repo_root)
}

#[test]
fn read_parse_schema_and_root_failures_keep_typed_identity() {
    let fixture = root("admission");
    let missing = fixture.join("missing.json");
    assert!(matches!(
        run(&options(&missing, &fixture)),
        Err(RunnerError::ReadManifest { path, .. }) if path == missing
    ));

    let malformed = write_manifest(&fixture, "{");
    assert!(matches!(
        run(&options(&malformed, &fixture)),
        Err(RunnerError::ParseManifest { path, .. }) if path == malformed
    ));

    let invalid_schema = write_manifest(
        &fixture,
        &manifest(
            vec![gate("fixture", &["true".to_owned()])],
            valid_profiles(&["fixture"]),
        )
        .replace("omnirepo.quality-manifest.v1", "other.schema"),
    );
    assert!(matches!(
        run(&options(&invalid_schema, &fixture)),
        Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("schema")
    ));

    let valid = write_manifest(
        &fixture,
        &manifest(
            vec![gate("fixture", &["true".to_owned()])],
            valid_profiles(&["fixture"]),
        ),
    );
    let missing_root = fixture.join("not-a-repository");
    assert!(matches!(
        run(&options(&valid, &missing_root)),
        Err(RunnerError::InvalidRepositoryRoot { path, .. }) if path == missing_root
    ));
    cleanup(&fixture);
}

#[test]
fn unknown_profile_is_rejected_before_any_gate_starts() {
    let fixture = root("profile-preflight");
    let marker = fixture.join("started");
    let command = format!("touch '{}'", marker.display());
    let gate_argv = vec!["/bin/sh".to_owned(), "-c".to_owned(), command];
    let manifest_path = write_manifest(
        &fixture,
        &manifest(
            vec![gate("fixture", &gate_argv)],
            valid_profiles(&["fixture"]),
        ),
    );

    let result = run(&options(&manifest_path, &fixture).with_profile("unknown"));
    assert!(matches!(
        result,
        Err(RunnerError::UnknownProfile { profile, .. }) if profile == "unknown"
    ));
    assert!(!marker.exists(), "preflight errors must not spawn a child");
    cleanup(&fixture);
}

#[test]
fn selected_gates_run_in_manifest_order_and_preserve_failures() {
    let fixture = root("ordered-run");
    let marker = fixture.join("execution.log");
    let command = |label: &str, exit_code: i32| {
        vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!(
                "printf '{}\\n' >> '{}'; exit {exit_code}",
                label,
                marker.display()
            ),
        ]
    };
    let gates = vec![
        gate("first", &command("first", 7)),
        gate("second", &command("second", 0)),
        gate("third", &command("third", 23)),
    ];
    let manifest_path = write_manifest(
        &fixture,
        &manifest(
            gates,
            vec![
                profile("full", &["first", "second", "third"]),
                profile("stable", &["second"]),
                profile("msrv", &["third"]),
                profile("coverage", &["first"]),
            ],
        ),
    );

    let report = run(&options(&manifest_path, &fixture)).expect("full profile runs");
    assert_eq!(report.profile, "full");
    assert!(!report.success);
    assert_eq!(report.exit_code, 1);
    assert_eq!(report.gates.len(), 3);
    assert_eq!(
        report
            .gates
            .iter()
            .map(|gate| gate.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third"]
    );
    assert_eq!(report.gates[0].exit_code, Some(7));
    assert_eq!(report.gates[1].exit_code, Some(0));
    assert_eq!(report.gates[2].exit_code, Some(23));
    assert!(!report.gates[0].success);
    assert!(report.gates[1].success);
    assert!(!report.gates[2].success);
    assert_eq!(
        fs::read_to_string(&marker).expect("all selected gates write the marker"),
        "first\nsecond\nthird\n"
    );

    fs::remove_file(&marker).expect("reset marker for explicit profile");
    let stable = run(&options(&manifest_path, &fixture).with_profile("stable"))
        .expect("stable profile runs");
    assert_eq!(stable.profile, "stable");
    assert!(stable.success);
    assert_eq!(stable.gates.len(), 1);
    assert_eq!(stable.gates[0].id, "second");
    assert_eq!(
        fs::read_to_string(&marker).expect("stable gate writes marker"),
        "second\n"
    );
    cleanup(&fixture);
}

#[test]
fn gate_result_deserializes_and_retains_all_stream_fields() {
    let json = serde_json::to_value(GateResult {
        id: "fixture".to_owned(),
        failure_identity: "quality.fixture".to_owned(),
        toolchain: "fixture-rust".to_owned(),
        working_directory: ".".to_owned(),
        exit_code: None,
        success: false,
        stdout: "replacement �".to_owned(),
        stderr: "diagnostic".to_owned(),
        failure: None,
    })
    .expect("serialize gate result");
    assert_eq!(json["id"], "fixture");
    assert_eq!(json["exit_code"], Value::Null);
    assert_eq!(json["stdout"], "replacement �");
    assert_eq!(json["stderr"], "diagnostic");
}
