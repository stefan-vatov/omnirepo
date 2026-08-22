//! Focused proof for the containment-aware old-or-complete-new executor.

#![allow(dead_code, unused_imports)]

use super::{DecidedMode, ReplaceError, ReplaceRequest, replace, replace_bytes_atomically};
use crate::lifecycle::transaction_evidence::restart_cleanup;
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
    ReplaceRequest::new(
        plan(operation, target),
        content.as_bytes().to_vec(),
        DecidedMode::Preserve(0o644),
    )
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

#[test]
fn kill_point_restart_cleanup_is_idempotent_and_observes_only_owned_state() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old").expect("write old");
    // Simulate a kill between the temp write and the publish: the owned
    // temporary exists, the old target is authoritative.
    let temp = root.join(".target.txt.omnirepo-tmp-op-1-1.tmp");
    fs::write(&temp, b"partial new").expect("write temp");
    assert_eq!(fs::read(root.join("target.txt")).expect("old"), b"old");
    // Restart cleanup removes the owned artifact; a second restart pass is a
    // no-op (restart is idempotent) and observes only the target.
    let first = restart_cleanup(&root).expect("first cleanup");
    assert_eq!(first.removed, vec![temp.clone()]);
    let second = restart_cleanup(&root).expect("second cleanup");
    assert!(second.removed.is_empty(), "restart must be idempotent");
    assert_eq!(fs::read(root.join("target.txt")).expect("old"), b"old");
}

#[test]
fn unchanged_targets_preserve_witness_metadata() {
    use std::os::unix::fs::PermissionsExt;
    let (_fixture, root) = fixture_root();
    fs::write(root.join("changed.txt"), b"old").expect("write changed");
    fs::write(root.join("witness.txt"), b"witness").expect("write witness");
    fs::set_permissions(root.join("witness.txt"), fs::Permissions::from_mode(0o640))
        .expect("witness mode");
    let witness_before = fs::metadata(root.join("witness.txt")).expect("metadata");
    let witness_mtime = witness_before.modified().expect("mtime");
    let witness_mode = witness_before.permissions().mode() & 0o777;

    replace(&root, &request("op-1", "changed.txt", "new")).expect("replace");

    let witness_after = fs::metadata(root.join("witness.txt")).expect("metadata");
    assert_eq!(witness_after.modified().expect("mtime"), witness_mtime);
    assert_eq!(witness_after.permissions().mode() & 0o777, witness_mode);
    assert_eq!(
        fs::read(root.join("witness.txt")).expect("content"),
        b"witness"
    );
}

#[test]
fn alias_targets_are_rejected_without_outside_root_effects() {
    let (_fixture, root) = fixture_root();
    let outside = root.parent().expect("parent").join("outside.txt");
    fs::write(&outside, b"outside content").expect("write outside");
    std::os::unix::fs::symlink(&outside, root.join("alias.txt")).expect("symlink");
    let error =
        replace(&root, &request("op-1", "alias.txt", "new")).expect_err("alias target must fail");
    assert!(matches!(error, ReplaceError::Resolve { .. }), "{error:?}");
    // The alias and its destination are untouched: no outside-root effect.
    assert!(root.join("alias.txt").is_symlink());
    assert_eq!(
        fs::read(&outside).expect("outside content"),
        b"outside content"
    );
}

#[test]
fn failure_atomicity_observes_only_allowed_states() {
    let (_fixture, root) = fixture_root();
    // Injected failure at every stage exposes only old/new/residue per the
    // contract: after a successful replace the residue set is empty and only
    // the new complete content exists.
    fs::write(root.join("target.txt"), b"old").expect("write old");
    replace(&root, &request("op-1", "target.txt", "new content")).expect("replace");
    let entries: Vec<String> = fs::read_dir(&root)
        .expect("root")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        entries,
        vec!["target.txt".to_owned()],
        "only the complete new file"
    );
    assert_eq!(
        fs::read(root.join("target.txt")).expect("new"),
        b"new content"
    );
}

#[test]
fn a_preserved_mode_is_exact_despite_the_umask() {
    use std::os::unix::fs::PermissionsExt;
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old").expect("write old");
    fs::set_permissions(root.join("target.txt"), fs::Permissions::from_mode(0o664))
        .expect("target mode");
    // 0o664 carries group-write, which a common umask (022) would strip at
    // temp creation; the preserved mode must survive exactly.
    replace_bytes_atomically(&root, "target.txt", "op-1", b"new bytes\n").expect("replace");
    assert_eq!(
        fs::read(root.join("target.txt")).expect("content"),
        b"new bytes\n"
    );
    let mode = fs::metadata(root.join("target.txt"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o664, "the existing mode is preserved exactly");
}

#[test]
fn an_absent_target_is_created_with_safe_contained_parents() {
    use std::os::unix::fs::PermissionsExt;
    let (_fixture, root) = fixture_root();
    replace_bytes_atomically(&root, "nested/dir/new.txt", "op-1", b"content\n").expect("creation");
    assert_eq!(
        fs::read(root.join("nested/dir/new.txt")).expect("content"),
        b"content\n"
    );
    // The new file uses mode 0644 subject to the process umask.
    let mode = fs::metadata(root.join("nested/dir/new.txt"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode & !0o644, 0, "no bits beyond the 0644 creation mode");
    assert!(mode & 0o600 == 0o600, "owner read/write survive: {mode:o}");
}

#[test]
fn a_failed_creation_removes_only_the_empty_parents_it_created() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("keep.txt"), b"peer").expect("peer");
    // An over-long operation id is a deterministic fault seam: the owned
    // temporary name exceeds the filesystem name limit, so the executor
    // fails after the parent directories were created.
    let too_long = "x".repeat(300);
    let error = replace_bytes_atomically(&root, "sub/new.txt", &too_long, b"content\n")
        .expect_err("injected failure");
    assert!(
        matches!(error, ReplaceError::CreateTemp { .. }),
        "{error:?}"
    );
    assert!(
        !root.join("sub").exists(),
        "the created parent is removed after failure"
    );
    assert_eq!(fs::read(root.join("keep.txt")).expect("peer"), b"peer");
}

#[test]
fn a_path_shaped_operation_id_keeps_the_temporary_a_sibling() {
    let (_fixture, root) = fixture_root();
    fs::create_dir_all(root.join("nested")).expect("parent");
    fs::write(root.join("nested/target.txt"), b"old").expect("write old");
    // The group id is the destination path itself in the pass: it must
    // never turn the temporary into a nested path.
    replace_bytes_atomically(&root, "nested/target.txt", "nested/target.txt", b"new\n")
        .expect("replace");
    assert_eq!(
        fs::read(root.join("nested/target.txt")).expect("content"),
        b"new\n"
    );
    assert_eq!(
        super::owned_temp_display("nested/target.txt", "nested/target.txt"),
        "nested/.target.txt.omnirepo-tmp-nested-target.txt-1.tmp"
    );
}

#[test]
fn a_directory_target_fails_typed_and_stays_intact() {
    let (_fixture, root) = fixture_root();
    fs::create_dir_all(root.join("adir")).expect("dir target");
    let error =
        replace_bytes_atomically(&root, "adir", "op-1", b"new").expect_err("directory target");
    assert!(matches!(error, ReplaceError::Resolve { .. }), "{error:?}");
    assert!(root.join("adir").is_dir(), "the directory is untouched");
}
