//! Focused proof for re-running the normative gates and verifying
//! candidate provenance.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_gates::{
    GateRun, MAX_GATE_OUTPUT_BYTES, ProvenanceError, run_normative_gates,
    run_normative_gates_with_budget, verify_candidate_provenance,
};
use crate::lifecycle::release_manifest::{CandidateManifest, manifest_for};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const HELPER_TEST_PREFIX: &str = "lifecycle::release_gates::release_gates_tests::";

fn helper_gate(name: &str) -> Vec<String> {
    vec![
        std::env::current_exe()
            .expect("test executable")
            .display()
            .to_string(),
        "--ignored".to_owned(),
        "--exact".to_owned(),
        format!("{HELPER_TEST_PREFIX}{name}"),
        "--nocapture".to_owned(),
    ]
}

#[test]
#[ignore]
fn passing_gate_helper() {}

#[test]
#[ignore]
fn hanging_gate_helper() {
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
#[ignore]
fn overflowing_gate_helper() {
    std::io::stdout()
        .write_all(&vec![b'x'; MAX_GATE_OUTPUT_BYTES + 1])
        .expect("write gate output");
}

#[test]
#[ignore]
#[allow(clippy::zombie_processes)]
fn pipe_holding_descendant_gate_helper() {
    // The gate runner owns and reaps this helper's complete process group.
    // Let the descendant outlive this direct parent to prove that boundary.
    Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--ignored",
            "--exact",
            &format!("{HELPER_TEST_PREFIX}hanging_gate_helper"),
            "--nocapture",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn pipe-holding descendant");
}

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-gates-")
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn the_gate_orchestrator_runs_every_gate_and_collects_the_results() {
    let fixture = fixture_base();
    // A fixture gate that passes and one that fails; both are collected.
    let pass = fixture.path().join("gate-pass");
    let fail = fixture.path().join("gate-fail");
    fs::write(&pass, "#!/bin/sh\nexit 0\n").expect("pass");
    fs::write(&fail, "#!/bin/sh\necho gate failed\nexit 1\n").expect("fail");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&pass, &fail] {
            let mut permissions = fs::metadata(path).expect("meta").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("mode");
        }
    }
    let gates = vec![
        ("fmt".to_owned(), vec![pass.display().to_string()]),
        ("clippy".to_owned(), vec![fail.display().to_string()]),
    ];
    let runs = run_normative_gates(&gates);
    assert_eq!(runs.len(), 2, "every gate is collected");
    assert!(runs[0].passed, "{:?}", runs[0]);
    assert!(!runs[1].passed, "{:?}", runs[1]);
    assert!(runs[1].evidence.contains("gate failed"), "{:?}", runs[1]);
}

#[test]
fn a_gate_spawn_failure_is_collected_without_skipping_later_gates() {
    let fixture = fixture_base();
    let gates = vec![
        (
            "missing".to_owned(),
            vec![fixture.path().join("absent").display().to_string()],
        ),
        ("pass".to_owned(), helper_gate("passing_gate_helper")),
    ];

    let runs = run_normative_gates(&gates);

    assert_eq!(runs.len(), 2, "every gate has a terminal result");
    assert!(!runs[0].passed);
    assert!(runs[0].evidence.contains("cannot start gate"));
    assert!(runs[1].passed, "a failed peer cannot stop a later gate");
}

#[test]
fn a_gate_deadline_is_bounded_without_skipping_later_gates() {
    let gates = vec![
        ("hang".to_owned(), helper_gate("hanging_gate_helper")),
        ("pass".to_owned(), helper_gate("passing_gate_helper")),
    ];

    let started = Instant::now();
    let runs = run_normative_gates_with_budget(&gates, Duration::from_millis(50));

    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(runs.len(), 2);
    assert!(!runs[0].passed);
    assert!(runs[0].evidence.contains("exceeded its"));
    assert!(runs[1].passed, "a timed-out gate cannot stop a later gate");
}

#[test]
fn excessive_gate_output_is_bounded_without_skipping_later_gates() {
    let gates = vec![
        (
            "overflow".to_owned(),
            helper_gate("overflowing_gate_helper"),
        ),
        ("pass".to_owned(), helper_gate("passing_gate_helper")),
    ];

    let runs = run_normative_gates_with_budget(&gates, Duration::from_secs(1));

    assert_eq!(runs.len(), 2);
    assert!(!runs[0].passed);
    assert!(runs[0].evidence.contains("output exceeded"));
    assert!(
        runs[1].passed,
        "an overflowing gate cannot stop a later gate"
    );
}

#[cfg(unix)]
#[test]
fn a_gate_cannot_leave_pipe_holding_descendants() {
    let gates = vec![(
        "descendant".to_owned(),
        helper_gate("pipe_holding_descendant_gate_helper"),
    )];

    let started = Instant::now();
    let runs = run_normative_gates_with_budget(&gates, Duration::from_secs(1));

    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(runs[0].passed, "{:?}", runs[0]);
}

#[test]
fn provenance_verifies_the_manifest_against_the_checkout() {
    let fixture = fixture_base();
    let checkout = fixture.path().join("checkout");
    fs::create_dir_all(&checkout).expect("checkout");
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let manifest =
        manifest_for("0.9.0", commit, "rustc 1.86.0", Vec::new(), Vec::new()).expect("manifest");
    let mut head_file = checkout.clone();
    head_file.push("HEAD");
    fs::write(&head_file, commit).expect("head");
    // The manifest provenance matches the checkout.
    assert!(verify_candidate_provenance(&manifest, &checkout).is_ok());
    // A checkout at a different commit fails typed.
    let other = fixture.path().join("other");
    fs::create_dir_all(&other).expect("other");
    fs::write(
        other.join("HEAD"),
        "fedcba9876543210fedcba9876543210fedcba98",
    )
    .expect("head");
    let error = verify_candidate_provenance(&manifest, &other).expect_err("mismatch");
    assert!(
        matches!(error, ProvenanceError::CommitMismatch { .. }),
        "{error}"
    );
}

#[test]
fn provenance_refuses_a_tampered_manifest() {
    let fixture = fixture_base();
    let checkout = fixture.path().join("checkout");
    fs::create_dir_all(&checkout).expect("checkout");
    let commit = "0123456789abcdef0123456789abcdef01234567";
    fs::write(checkout.join("HEAD"), commit).expect("head");
    // A manifest whose content hash was replaced no longer matches its
    // own identity.
    let mut manifest =
        manifest_for("0.9.0", commit, "rustc 1.86.0", Vec::new(), Vec::new()).expect("manifest");
    manifest.identity.manifest_sha256 = "deadbeef".to_owned();
    let error = verify_candidate_provenance(&manifest, &checkout).expect_err("tampered");
    assert!(
        matches!(error, ProvenanceError::ManifestTampered { .. }),
        "{error}"
    );
}

#[test]
fn a_missing_checkout_head_fails_typed() {
    let fixture = fixture_base();
    let checkout = fixture.path().join("missing");
    let manifest = manifest_for(
        "0.9.0",
        "0123456789abcdef0123456789abcdef01234567",
        "rustc 1.86.0",
        Vec::new(),
        Vec::new(),
    )
    .expect("manifest");
    let error = verify_candidate_provenance(&manifest, &checkout).expect_err("missing");
    assert!(
        matches!(error, ProvenanceError::HeadUnavailable { .. }),
        "{error}"
    );
}
