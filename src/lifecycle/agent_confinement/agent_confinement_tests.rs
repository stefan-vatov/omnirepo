//! Focused proof for destination-only agent confinement.

#![allow(dead_code, unused_imports)]

use super::{ConfinementError, confine};
use std::{collections::HashMap, fs, path::Path, path::PathBuf};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("confinement-home-")
        .tempdir_in(&base)
        .expect("fixture");
    let destination = fixture.path().join("destination");
    fs::create_dir_all(&destination).expect("destination");
    let inside = destination.join("tools");
    fs::create_dir_all(&inside).expect("inside");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&outside).expect("outside");
    (fixture, destination, inside, outside)
}

#[test]
fn agent_confinement_is_destination_only() {
    let (_fixture, destination, inside, _outside) = fixture();
    let confinement = confine(&destination, &[inside.clone()], &[inside.clone()]).expect("confine");
    assert_eq!(
        confinement.workdir,
        destination.canonicalize().expect("canonical")
    );
    let env: HashMap<String, String> = confinement.env.into_iter().collect();
    assert_eq!(
        env.get("HOME").expect("home"),
        destination
            .canonicalize()
            .expect("canonical")
            .to_str()
            .expect("utf8")
    );
    assert!(
        env.get("TMPDIR").expect("tmpdir").contains("omnirepo-tmp"),
        "{env:?}"
    );
    assert!(env.get("PATH").expect("path").contains("tools"), "{env:?}");
}

#[test]
fn outside_extra_paths_escape_and_fail() {
    let (_fixture, destination, _inside, outside) = fixture();
    let error = confine(&destination, &[], &[outside.clone()]).expect_err("escape must fail");
    assert!(
        matches!(error, ConfinementError::EscapesDestination { .. }),
        "{error:?}"
    );
}

#[test]
fn missing_destination_is_a_typed_root_error() {
    let (_fixture, _destination, _inside, _outside) = fixture();
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let missing = base.join("confinement-missing-destination");
    let _ = fs::remove_dir_all(&missing);
    let error = confine(&missing, &[], &[]).expect_err("missing root must fail");
    assert!(matches!(error, ConfinementError::Root { .. }), "{error:?}");
}
