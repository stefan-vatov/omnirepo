//! Focused proof for the post-agent delta and residue policy.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_delta::{
    DeltaVerdict, DirSnapshot, classify_post_agent_delta, snapshot_dir,
};
use std::{fs, path::Path};

fn fixture_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("repair-delta-")
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn an_unchanged_destination_classifies_as_no_delta() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.txt"), "v1\n").expect("file");
    let before = snapshot_dir(fixture.path()).expect("before");
    let after = snapshot_dir(fixture.path()).expect("after");
    assert!(matches!(
        classify_post_agent_delta(&before, &after, &[]),
        DeltaVerdict::NoDelta
    ));
}

#[test]
fn a_changed_managed_file_classifies_as_expected_delta() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.txt"), "v1\n").expect("file");
    let before = snapshot_dir(fixture.path()).expect("before");
    fs::write(fixture.path().join("managed.txt"), "v2\n").expect("changed");
    let after = snapshot_dir(fixture.path()).expect("after");
    let verdict = classify_post_agent_delta(&before, &after, &[]);
    match verdict {
        DeltaVerdict::ExpectedDelta { changed } => {
            assert!(changed.iter().any(|p| p == "managed.txt"), "{changed:?}");
        }
        other => panic!("expected expected-delta, got {other:?}"),
    }
}

#[test]
fn an_unexpected_residue_file_fails_typed() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.txt"), "v1\n").expect("file");
    let before = snapshot_dir(fixture.path()).expect("before");
    fs::write(fixture.path().join("leftover.bin"), "x").expect("residue");
    let after = snapshot_dir(fixture.path()).expect("after");
    let verdict = classify_post_agent_delta(&before, &after, &[]);
    match verdict {
        DeltaVerdict::Residue { paths } => {
            assert!(paths.iter().any(|p| p == "leftover.bin"), "{paths:?}");
        }
        other => panic!("expected residue, got {other:?}"),
    }
}

#[test]
fn an_allowed_residue_is_tolerated() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.txt"), "v1\n").expect("file");
    let before = snapshot_dir(fixture.path()).expect("before");
    fs::write(fixture.path().join("evidence.out"), "ok").expect("allowed");
    let after = snapshot_dir(fixture.path()).expect("after");
    assert!(matches!(
        classify_post_agent_delta(&before, &after, &["evidence.out".to_owned()]),
        DeltaVerdict::ExpectedDelta { .. } | DeltaVerdict::NoDelta
    ));
}

#[test]
fn a_missing_destination_fails_typed() {
    let fixture = fixture_root();
    let missing = fixture.path().join("absent");
    assert!(snapshot_dir(&missing).is_err());
}
