//! Focused proof for external verifier confinement.

#![allow(dead_code, unused_imports)]

use super::{
    ArtifactDisposition, ConfineError, confine_verifier, confinement_evidence, dispositions,
};
use std::{fs, path::Path};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("verifier-confine-")
        .tempdir_in(&base)
        .expect("fixture");
    let destination = fixture.path().join("destination");
    (fixture, destination)
}

#[test]
fn confinement_opens_the_typed_destination_root() {
    let (_fixture, destination) = fixture();
    fs::create_dir_all(&destination).expect("destination");
    let confinement =
        confine_verifier(&destination, &["tmp/artifact".to_owned()], false).expect("confine");
    assert_eq!(
        confinement.destination.display_path().as_path(),
        &destination
    );
    // A missing destination fails before execution.
    let missing = fixture_path();
    let _ = fs::remove_dir_all(&missing);
    let error = confine_verifier(&missing, &[], false).expect_err("missing root");
    assert!(matches!(error, ConfineError::Root { .. }), "{error}");
}

#[test]
fn forbidden_ephemeral_paths_fail_closed() {
    let (_fixture, destination) = fixture();
    fs::create_dir_all(&destination).expect("destination");
    // Absolute and escaping artifacts are forbidden.
    let error =
        confine_verifier(&destination, &["/etc/passwd".to_owned()], false).expect_err("absolute");
    assert!(
        matches!(error, ConfineError::EphemeralOutsideRoot { .. }),
        "{error}"
    );
    let error =
        confine_verifier(&destination, &["../escape".to_owned()], false).expect_err("escaping");
    assert!(
        matches!(error, ConfineError::EphemeralOutsideRoot { .. }),
        "{error}"
    );
    // Duplicates fail typed.
    let error = confine_verifier(
        &destination,
        &["a.txt".to_owned(), "a.txt".to_owned()],
        false,
    )
    .expect_err("duplicate");
    assert!(
        matches!(error, ConfineError::DuplicateEphemeral { .. }),
        "{error}"
    );
}

#[test]
fn disposition_follows_the_retain_policy() {
    let (_fixture, destination) = fixture();
    fs::create_dir_all(&destination).expect("destination");
    let cleaned = confine_verifier(&destination, &["a.txt".to_owned()], false).expect("confine");
    assert_eq!(
        dispositions(&cleaned),
        vec![ArtifactDisposition::Cleaned {
            path: "a.txt".to_owned()
        }]
    );
    let retained = confine_verifier(&destination, &["a.txt".to_owned()], true).expect("confine");
    assert_eq!(
        dispositions(&retained),
        vec![ArtifactDisposition::Retained {
            path: "a.txt".to_owned()
        }]
    );
}

#[test]
fn evidence_is_bounded_and_secret_free() {
    let (_fixture, destination) = fixture();
    fs::create_dir_all(&destination).expect("destination");
    let confinement = confine_verifier(&destination, &["a.txt".to_owned()], true).expect("confine");
    let evidence = confinement_evidence(&confinement);
    assert!(evidence.contains("verifier-confinement"), "{evidence}");
    assert!(evidence.contains("ephemeral=1"), "{evidence}");
    assert!(evidence.contains("retain=true"), "{evidence}");
}

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("verifier-confine-missing")
}
