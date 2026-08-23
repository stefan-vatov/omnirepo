//! Focused proof for re-running the normative gates and verifying
//! candidate provenance.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_gates::{
    GateRun, ProvenanceError, run_normative_gates, verify_candidate_provenance,
};
use crate::lifecycle::release_manifest::{CandidateManifest, manifest_for};
use std::{fs, path::Path};

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
    let pass = fixture.path().join("gate-pass");
    fs::write(&pass, "#!/bin/sh\nexit 0\n").expect("pass");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&pass).expect("meta").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&pass, permissions).expect("mode");
    }
    let gates = vec![
        (
            "missing".to_owned(),
            vec![fixture.path().join("absent").display().to_string()],
        ),
        ("pass".to_owned(), vec![pass.display().to_string()]),
    ];

    let runs = run_normative_gates(&gates);

    assert_eq!(runs.len(), 2, "every gate has a terminal result");
    assert!(!runs[0].passed);
    assert!(runs[0].evidence.contains("cannot start gate"));
    assert!(runs[1].passed, "a failed peer cannot stop a later gate");
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
