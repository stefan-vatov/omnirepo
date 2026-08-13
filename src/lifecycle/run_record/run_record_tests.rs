//! Focused proof for the invocation-boundary run-record effect.

#![allow(dead_code, unused_imports)]

use super::{
    RunId, RunRecord, RunRecordError, map_create_error, map_parent_error,
    validate_absolute_directory_path,
};
use crate::platform::{
    PathError, test_creation_mode, test_durability_phase, test_reset_observations,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::{Builder, TempDir};

const FIXED_SUFFIX: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

fn fixed_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_754_064_000)
}

struct FixtureHome {
    tempdir: TempDir,
}

impl FixtureHome {
    fn path(&self) -> &Path {
        self.tempdir.path()
    }
}

impl Drop for FixtureHome {
    fn drop(&mut self) {
        restore_permissions_for_cleanup(self.path());
    }
}

fn restore_permissions_for_cleanup(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }

    if metadata.is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                restore_permissions_for_cleanup(&entry.path());
            }
        }
    } else if metadata.nlink() == 1 {
        // Never chmod a hard-linked file: its other name may be outside this
        // fixture root and must not be changed by cleanup.
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

fn fixture_home() -> FixtureHome {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&target).expect("create filesystem fixture base");
    let home = Builder::new()
        .prefix("run-record-home-")
        .tempdir_in(target)
        .expect("create filesystem fixture home");
    fs::create_dir_all(home.path().join(".omnirepo/runs")).expect("create run-record parent");
    eprintln!("run-record fixture: home={:?}", home.path());
    FixtureHome { tempdir: home }
}

fn create_fixed(home: &Path) -> Result<RunRecord, RunRecordError> {
    RunRecord::create_with_id(home, fixed_time(), FIXED_SUFFIX)
}

#[test]
fn creates_exclusive_private_versioned_intent_record() {
    let home = fixture_home();
    let record = create_fixed(home.path()).expect("initial record is created");
    let expected_name = "20250801T160000Z-000102030405060708090a0b0c0d0e0f.log";

    assert_eq!(
        record.path(),
        home.path().join(".omnirepo/runs").join(expected_name)
    );
    assert_eq!(
        record.id().to_string(),
        expected_name.trim_end_matches(".log")
    );

    let metadata = fs::metadata(record.path()).expect("record metadata is readable");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        metadata.len(),
        fs::read(record.path()).expect("read record").len() as u64
    );

    let contents = fs::read_to_string(record.path()).expect("read JSONL intent");
    assert_eq!(
        contents,
        "{\"version\":1,\"type\":\"run_intent\",\"run_id\":\"20250801T160000Z-000102030405060708090a0b0c0d0e0f\",\"created_at\":\"20250801T160000Z\",\"stage\":\"invocation\",\"status\":\"started\"}\n",
        "the first JSONL record has stable field order and exact bytes"
    );
}

#[test]
fn creates_with_private_mode_at_syscall_and_syncs_directory_after_file() {
    test_reset_observations();
    let home = fixture_home();

    let _record = create_fixed(home.path()).expect("record is created");

    assert_eq!(
        test_creation_mode(),
        0o600,
        "openat must request private mode before any post-create chmod"
    );
    assert_eq!(
        test_durability_phase(),
        2,
        "file sync must complete before parent directory sync"
    );
}

#[test]
fn operating_system_entropy_produces_a_full_128_bit_suffix() {
    let home = fixture_home();
    let record = RunRecord::create(home.path()).expect("OS entropy creates a record");

    assert_eq!(record.id().suffix().len(), 32);
    assert!(
        record
            .id()
            .suffix()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn repeated_identity_is_a_collision_and_never_overwrites() {
    let home = fixture_home();
    let first = create_fixed(home.path()).expect("first record is created");
    let before = fs::read(first.path()).expect("read first record");
    let error = create_fixed(home.path()).expect_err("same identity must collide");

    assert!(matches!(error, RunRecordError::Collision { .. }));
    assert_eq!(
        fs::read(first.path()).expect("read surviving record"),
        before
    );
}

#[test]
fn hostile_symlink_parent_is_rejected_without_following() {
    let home = fixture_home();
    let runs = home.path().join(".omnirepo/runs");
    let real = home.path().join("real-runs");
    fs::rename(&runs, &real).expect("move real run parent");
    std::os::unix::fs::symlink(&real, &runs).expect("install hostile parent symlink");

    let error = create_fixed(home.path()).expect_err("symlink parent must fail closed");
    assert!(matches!(error, RunRecordError::ParentRejected { .. }));
    assert!(
        fs::read_dir(real)
            .expect("inspect real parent")
            .next()
            .is_none()
    );
}

#[test]
fn non_regular_parent_is_rejected_without_creating_a_record() {
    let home = fixture_home();
    let runs = home.path().join(".omnirepo/runs");
    fs::remove_dir(&runs).expect("remove run directory");
    fs::write(&runs, b"not a directory").expect("create non-regular parent target");

    let error = create_fixed(home.path()).expect_err("non-directory parent must fail closed");
    assert!(matches!(error, RunRecordError::ParentRejected { .. }));
    assert_eq!(
        fs::read(&runs).expect("read non-directory target"),
        b"not a directory"
    );
}

#[test]
fn unavailable_home_and_parent_are_typed_pre_record_failures() {
    let root = fixture_home();
    let missing_home = root.path().join("missing-home");
    let home_error = create_fixed(&missing_home).expect_err("missing HOME must fail");
    assert!(matches!(
        home_error,
        RunRecordError::ParentUnavailable { .. }
    ));

    let no_parent = Builder::new()
        .prefix("run-record-no-parent-")
        .tempdir_in(root.path().parent().expect("fixture parent"))
        .expect("create home without run parent");
    let parent_error = create_fixed(no_parent.path()).expect_err("missing parent must fail");
    assert!(matches!(
        parent_error,
        RunRecordError::ParentUnavailable { .. }
    ));
}

#[test]
fn pre_record_failure_does_not_run_a_subsequent_effect() {
    let home = fixture_home();
    let _record = create_fixed(home.path()).expect("seed collision record");
    let mut subsequent_effect_ran = false;

    let result = create_fixed(home.path());
    if result.is_ok() {
        subsequent_effect_ran = true;
    }
    assert!(matches!(result, Err(RunRecordError::Collision { .. })));
    assert!(
        !subsequent_effect_ran,
        "record failure must stop later effects"
    );
}

#[test]
fn injected_identity_is_deterministic_and_does_not_depend_on_completion_order() {
    let first_home = fixture_home();
    let second_home = fixture_home();
    let first = create_fixed(first_home.path()).expect("first deterministic record");
    let second = create_fixed(second_home.path()).expect("second deterministic record");

    assert_eq!(first.id(), second.id());
    assert_eq!(
        first.path().file_name(),
        second.path().file_name(),
        "identity is independent of fixture completion order"
    );
}

#[test]
fn relative_home_is_rejected_before_parent_access() {
    let error = create_fixed(Path::new("relative-home")).expect_err("relative HOME must fail");
    assert!(matches!(error, RunRecordError::InvalidHome { .. }));
}

#[test]
fn pre_epoch_clock_is_a_typed_pre_record_failure() {
    let home = fixture_home();
    let error = RunRecord::create_with_id(
        home.path(),
        UNIX_EPOCH - Duration::from_secs(1),
        FIXED_SUFFIX,
    )
    .expect_err("pre-epoch clock must fail before opening the parent");
    assert!(matches!(error, RunRecordError::Clock { .. }));
}

#[test]
fn record_path_has_no_unexpected_components() {
    let home = fixture_home();
    let record = create_fixed(home.path()).expect("record is created");
    let relative = record
        .path()
        .strip_prefix(home.path())
        .expect("record is below HOME")
        .to_path_buf();
    assert_eq!(
        relative,
        PathBuf::from(".omnirepo/runs").join(record.id().file_name())
    );
    assert!(
        !relative
            .components()
            .any(|component| { matches!(component, std::path::Component::ParentDir) })
    );
}

#[test]
fn created_record_timestamps_cover_epoch_day_leap_and_fixed_dates() {
    let cases = [
        (UNIX_EPOCH, "19700101T000000Z"),
        (UNIX_EPOCH + Duration::from_secs(86_400), "19700102T000000Z"),
        (
            UNIX_EPOCH + Duration::from_secs(951_782_400),
            "20000229T000000Z",
        ),
        (fixed_time(), "20250801T160000Z"),
    ];

    for (timestamp, expected) in cases {
        let home = fixture_home();
        let record = RunRecord::create_with_id(home.path(), timestamp, FIXED_SUFFIX)
            .expect("timestamp creates a record");
        assert_eq!(record.id().timestamp(), expected);
    }
}

#[test]
fn absolute_directory_validation_rejects_relative_and_traversal_paths() {
    assert!(matches!(
        validate_absolute_directory_path(Path::new("relative")),
        Err(RunRecordError::InvalidHome { reason, .. }) if reason == "authority path must be absolute"
    ));
    assert!(matches!(
        validate_absolute_directory_path(Path::new("/virtual/../runs")),
        Err(RunRecordError::InvalidHome { reason, .. }) if reason == "authority path cannot contain parent traversal"
    ));
    assert!(validate_absolute_directory_path(Path::new("/virtual/runs")).is_ok());
}

#[test]
fn direct_directory_creation_validates_root_before_touching_the_filesystem() {
    let id = RunId::from_parts(fixed_time(), FIXED_SUFFIX).expect("fixed identity formats");

    let relative = RunRecord::create_in_directory(Path::new("relative-runs"), id.clone())
        .expect_err("relative run directory must fail before access");
    assert!(matches!(relative, RunRecordError::InvalidHome { .. }));

    let traversal = RunRecord::create_in_directory(Path::new("/virtual/../runs"), id)
        .expect_err("traversing run directory must fail before access");
    assert!(matches!(traversal, RunRecordError::InvalidHome { .. }));
}

#[test]
fn malformed_existing_record_is_a_collision_and_remains_unchanged() {
    let home = fixture_home();
    let path = home
        .path()
        .join(".omnirepo/runs/20250801T160000Z-000102030405060708090a0b0c0d0e0f.log");
    let malformed = b"not-json\ntruncated";
    fs::write(&path, malformed).expect("seed malformed record");

    let error = create_fixed(home.path()).expect_err("occupied identity must fail closed");
    assert!(matches!(error, RunRecordError::Collision { path: actual } if actual == path));
    assert_eq!(fs::read(path).expect("read malformed residue"), malformed);
}

#[test]
fn leaf_symlink_and_hard_link_are_rejected_without_outside_root_effects() {
    let symlink_home = fixture_home();
    let symlink_runs = symlink_home.path().join(".omnirepo/runs");
    let symlink_target = symlink_home.path().join("outside-record");
    let symlink_path = symlink_runs.join("20250801T160000Z-000102030405060708090a0b0c0d0e0f.log");
    let outside = b"outside sentinel";
    fs::write(&symlink_target, outside).expect("seed outside target");
    std::os::unix::fs::symlink(&symlink_target, &symlink_path).expect("seed leaf symlink");

    let symlink_error = create_fixed(symlink_home.path()).expect_err("leaf symlink is rejected");
    assert!(matches!(
        symlink_error,
        RunRecordError::ParentRejected { .. }
    ));
    assert_eq!(
        fs::read(&symlink_target).expect("read outside target"),
        outside
    );

    let hard_link_home = fixture_home();
    let hard_link_runs = hard_link_home.path().join(".omnirepo/runs");
    let hard_link_target = hard_link_home.path().join("outside-hard-link");
    let hard_link_path =
        hard_link_runs.join("20250801T160000Z-000102030405060708090a0b0c0d0e0f.log");
    fs::write(&hard_link_target, outside).expect("seed hard-link target");
    fs::hard_link(&hard_link_target, &hard_link_path).expect("seed leaf hard link");

    let hard_link_error = create_fixed(hard_link_home.path()).expect_err("hard link is rejected");
    assert!(matches!(
        hard_link_error,
        RunRecordError::ParentRejected { .. }
    ));
    assert_eq!(
        fs::read(&hard_link_target).expect("read hard-link target"),
        outside
    );
}

#[test]
fn leaf_directory_is_rejected_without_replacing_the_directory() {
    let home = fixture_home();
    let path = home
        .path()
        .join(".omnirepo/runs/20250801T160000Z-000102030405060708090a0b0c0d0e0f.log");
    fs::create_dir(&path).expect("seed directory at record identity");

    let error = create_fixed(home.path()).expect_err("directory leaf must fail closed");
    assert!(matches!(error, RunRecordError::ParentRejected { .. }));
    assert!(path.is_dir(), "rejected directory must remain a directory");
}

#[test]
fn cleanup_restores_damaged_modes_and_removes_the_owned_fixture_root() {
    let home = fixture_home();
    let root = home.path().to_path_buf();
    let runs = root.join(".omnirepo/runs");
    let record = runs.join("damaged.log");
    fs::write(&record, b"residue").expect("seed cleanup residue");
    fs::set_permissions(&record, fs::Permissions::from_mode(0o000))
        .expect("damage file mode for cleanup proof");
    fs::set_permissions(&runs, fs::Permissions::from_mode(0o000))
        .expect("damage nested directory mode for cleanup proof");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o000))
        .expect("damage fixture root mode for cleanup proof");

    drop(home);

    assert!(
        !root.exists(),
        "the cleanup guard must restore permissions before TempDir removes the fixture"
    );
}

#[test]
fn parent_error_mapping_keeps_typed_failures_and_exact_reasons() {
    let path = Path::new("/virtual/runs");
    let cases = [
        (
            PathError::NotFound {
                path: "missing".to_owned(),
            },
            "run-record parent unavailable \"/virtual/runs\": missing",
        ),
        (
            PathError::LinkLikeObject {
                path: "link".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs\": link",
        ),
        (
            PathError::MountCrossing {
                path: "mount".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs\": mount",
        ),
        (
            PathError::InvalidAuthorityRoot {
                path: "root".to_owned(),
                reason: "not a directory".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs\": root",
        ),
        (
            PathError::UnsupportedFilesystem {
                path: "root".to_owned(),
                kind: "unsupported-fixture-fs".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs\": root",
        ),
        (
            PathError::InvalidAbsolutePath {
                path: "root".to_owned(),
                reason: "bad root".to_owned(),
            },
            "run-record creation failed for \"/virtual/runs\": invalid absolute path \"root\": bad root",
        ),
    ];

    for (error, expected) in cases {
        let mapped = map_parent_error(path, error);
        assert_eq!(mapped.to_string(), expected);
    }
}

#[test]
fn create_error_mapping_distinguishes_collision_security_permission_and_other_failures() {
    let path = Path::new("/virtual/runs/record.log");
    let cases = [
        (
            PathError::Io {
                operation: "create".to_owned(),
                path: "record.log".to_owned(),
                kind: "exists".to_owned(),
                code: Some(17),
            },
            "run-record path already exists: \"/virtual/runs/record.log\"",
        ),
        (
            PathError::LinkLikeObject {
                path: "record.log".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs/record.log\": link-like object rejected without following: \"record.log\"",
        ),
        (
            PathError::MountCrossing {
                path: "record.log".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs/record.log\": filesystem boundary crossed below authority root: \"record.log\"",
        ),
        (
            PathError::UnsafeHardLink {
                path: "record.log".to_owned(),
                links: 2,
            },
            "run-record parent rejected \"/virtual/runs/record.log\": mutation target \"record.log\" has 2 hard-link names and is unsafe",
        ),
        (
            PathError::Io {
                operation: "create".to_owned(),
                path: "record.log".to_owned(),
                kind: "permission denied".to_owned(),
                code: Some(13),
            },
            "run-record permissions failed for \"/virtual/runs/record.log\": create failed for authority path \"record.log\": permission denied (errno=Some(13))",
        ),
        (
            PathError::Io {
                operation: "create".to_owned(),
                path: "record.log".to_owned(),
                kind: "operation failed".to_owned(),
                code: None,
            },
            "run-record creation failed for \"/virtual/runs/record.log\": create failed for authority path \"record.log\": operation failed (errno=None)",
        ),
    ];

    for (error, expected) in cases {
        let mapped = map_create_error(path, error);
        assert_eq!(mapped.to_string(), expected);
    }
}

#[test]
fn every_public_record_error_has_stable_display_text() {
    let path = PathBuf::from("/virtual/runs/record.log");
    let cases = [
        (
            RunRecordError::InvalidHome {
                path: path.clone(),
                reason: "HOME must be absolute",
            },
            "invalid HOME \"/virtual/runs/record.log\": HOME must be absolute",
        ),
        (
            RunRecordError::ParentUnavailable {
                path: path.clone(),
                reason: "missing".to_owned(),
            },
            "run-record parent unavailable \"/virtual/runs/record.log\": missing",
        ),
        (
            RunRecordError::ParentRejected {
                path: path.clone(),
                reason: "link".to_owned(),
            },
            "run-record parent rejected \"/virtual/runs/record.log\": link",
        ),
        (
            RunRecordError::Collision { path: path.clone() },
            "run-record path already exists: \"/virtual/runs/record.log\"",
        ),
        (
            RunRecordError::Create {
                path: path.clone(),
                reason: "create failed".to_owned(),
            },
            "run-record creation failed for \"/virtual/runs/record.log\": create failed",
        ),
        (
            RunRecordError::Permission {
                path: path.clone(),
                reason: "denied".to_owned(),
            },
            "run-record permissions failed for \"/virtual/runs/record.log\": denied",
        ),
        (
            RunRecordError::Write {
                path,
                reason: "disk".to_owned(),
            },
            "run-record write failed for \"/virtual/runs/record.log\": disk",
        ),
        (
            RunRecordError::Clock {
                reason: "before epoch".to_owned(),
            },
            "run-record clock failed: before epoch",
        ),
        (
            RunRecordError::Entropy {
                reason: "unavailable".to_owned(),
            },
            "run-record entropy failed: unavailable",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}
