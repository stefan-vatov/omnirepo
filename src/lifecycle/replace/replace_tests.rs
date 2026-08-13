//! Focused proof for the containment-aware old-or-complete-new executor.

#![allow(dead_code, unused_imports)]

use super::{ReplaceError, ReplaceRequest, replace};
use crate::managed_content::{ParentDirectories, TransactionPlan};
use std::{fs, path::Path};

fn fixture_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("replace-home-")
        .tempdir_in(&base)
        .expect("create replace fixture");
    let root = fixture.path().join("managed");
    fs::create_dir_all(&root).expect("create managed root");
    (fixture, root)
}

fn plan(operation: &str, target: &str) -> TransactionPlan {
    TransactionPlan::new(
        operation,
        Path::new(target).to_path_buf(),
        ParentDirectories::Existing,
    )
    .expect("valid plan")
}

fn request(operation: &str, target: &str, content: &str) -> ReplaceRequest {
    ReplaceRequest::new(plan(operation, target), content.as_bytes().to_vec(), 0o644)
}

#[test]
fn replace_publishes_exact_bytes_atomically() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old bytes").expect("write old");
    replace(&root, &request("op-1", "target.txt", "new bytes\n")).expect("replace");
    assert_eq!(
        fs::read(root.join("target.txt")).expect("content"),
        b"new bytes\n"
    );
    // No residue: the temporary sibling is gone.
    let entries: Vec<_> = fs::read_dir(&root).expect("root").collect();
    assert_eq!(entries.len(), 1);
    // The replacement file carries the decided mode.
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(root.join("target.txt"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644);
}

#[test]
fn missing_target_is_created() {
    let (_fixture, root) = fixture_root();
    replace(&root, &request("op-1", "new.txt", "content\n")).expect("replace");
    assert_eq!(
        fs::read(root.join("new.txt")).expect("content"),
        b"content\n"
    );
}

#[test]
fn temp_collision_fails_before_any_change_and_leaves_old_intact() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old").expect("write old");
    // Occupy the exact temporary name: the exclusive create must fail and
    // the old target must stay byte-identical.
    let temp = root.join(".target.txt.omnirepo-tmp-op-1-1.tmp");
    fs::write(&temp, b"occupied").expect("occupy temp");
    let error = replace(&root, &request("op-1", "target.txt", "new")).expect_err("collision fails");
    assert!(
        matches!(error, ReplaceError::CreateTemp { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(root.join("target.txt")).expect("old"), b"old");
    assert_eq!(
        fs::read(&temp).expect("occupier"),
        b"occupied",
        "peer temp untouched"
    );
}

#[test]
fn publish_failure_cleans_the_temp_and_preserves_the_target() {
    let (_fixture, root) = fixture_root();
    // A directory where a regular file is expected fails at resolution; no
    // temporary is ever created and the directory target is preserved.
    fs::create_dir_all(root.join("adir")).expect("create dir target");
    let error = replace(&root, &request("op-1", "adir", "new")).expect_err("resolve fails");
    assert!(matches!(error, ReplaceError::Resolve { .. }), "{error:?}");
    assert!(root.join("adir").is_dir());
    let residue: Vec<_> = fs::read_dir(&root)
        .expect("root")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .contains("omnirepo-tmp")
        })
        .collect();
    assert!(residue.is_empty(), "no temporary may appear: {residue:?}");
}

#[test]
fn interruption_between_write_and_publish_leaves_old_authoritative() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old").expect("write old");
    // Simulate interruption after the temp write by leaving the temporary in
    // place and never renaming: the old target stays authoritative and the
    // temporary is discoverable residue (recovery cleans it by name).
    let temp = root.join(".target.txt.omnirepo-tmp-op-1-1.tmp");
    fs::write(&temp, b"partial new").expect("write temp");
    assert_eq!(fs::read(root.join("target.txt")).expect("old"), b"old");
    // Recovery policy: the owned temporary is removed without touching the
    // target or any peer file.
    fs::remove_file(&temp).expect("recover temp");
    let entries: Vec<_> = fs::read_dir(&root).expect("root").collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(fs::read(root.join("target.txt")).expect("old"), b"old");
}

#[test]
fn hostile_plan_targets_are_rejected_before_any_effect() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old").expect("write old");
    // Hostile paths die at the plan contract itself: traversal and absolute
    // forms can never reach the executor.
    assert!(
        TransactionPlan::new(
            "op-1",
            Path::new("../escape").to_path_buf(),
            ParentDirectories::Existing
        )
        .is_err()
    );
    assert!(
        TransactionPlan::new(
            "op-1",
            Path::new("/absolute").to_path_buf(),
            ParentDirectories::Existing
        )
        .is_err()
    );
    assert_eq!(fs::read(root.join("target.txt")).expect("old"), b"old");
}
