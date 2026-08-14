//! Focused proof for the pre-attempt repository snapshot and frozen-input
//! verification.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_snapshot::{
    SnapshotError, snapshot_pre_attempt, verify_frozen_inputs,
};
use std::{fs, path::Path, process::Command};

fn git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-snapshot-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("repo");
    fs::create_dir_all(&root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Commit"]);
    git(&["config", "user.email", "commit@example.test"]);
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    (fixture, root)
}

#[test]
fn the_pre_attempt_snapshot_records_head_and_managed_identity() {
    let (_fixture, root) = git_repo();
    let snapshot = snapshot_pre_attempt(&root, "baseline-1", "lineage-1").expect("snapshot");
    assert_eq!(snapshot.repository, root.to_str().expect("path"));
    assert_eq!(snapshot.baseline_identity, "baseline-1");
    assert_eq!(snapshot.frozen_lineage_identity, "lineage-1");
    assert!(!snapshot.head_oid.is_empty(), "the head OID is recorded");
    assert!(
        !snapshot.managed_identity.is_empty(),
        "the managed identity is recorded"
    );
}

#[test]
fn frozen_inputs_are_verified_against_the_snapshot() {
    let (_fixture, root) = git_repo();
    let snapshot = snapshot_pre_attempt(&root, "baseline-1", "lineage-1").expect("snapshot");
    // The frozen inputs match the snapshot: verification passes.
    assert!(verify_frozen_inputs(
        &snapshot,
        &["baseline-1".to_owned(), "lineage-1".to_owned()]
    ));
    // A mismatched frozen input fails verification.
    assert!(!verify_frozen_inputs(&snapshot, &["baseline-9".to_owned()]));
    assert!(!verify_frozen_inputs(&snapshot, &[]));
}

#[test]
fn a_missing_repository_fails_typed() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let missing = base.join("repair-snapshot-missing");
    let _ = fs::remove_dir_all(&missing);
    let error = snapshot_pre_attempt(&missing, "b", "l").expect_err("missing repo");
    assert!(matches!(error, SnapshotError::Root { .. }), "{error}");
}
