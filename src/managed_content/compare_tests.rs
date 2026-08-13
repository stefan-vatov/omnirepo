//! Focused proof for exact compare and unchanged no-op detection.
//!
//! The caller-side platform read is exercised in the test fixtures (tests may
//! use the authority); compare itself is a pure exact-bytes decision.

#![allow(dead_code, unused_imports)]

use super::compare::{CompareError, CompareOutcome, compare};
use super::transaction::{ParentDirectories, TransactionPlan};
use crate::platform::{DestinationRepositoryRoot, ObjectClass, RelativePath, open_read_root};
use std::{fs, io::Read, path::Path, path::PathBuf};

fn fixture_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("compare-home-")
        .tempdir_in(&base)
        .expect("create compare fixture");
    let root = fixture.path().join("managed");
    fs::create_dir_all(&root).expect("create managed root");
    (fixture, root)
}

fn read_current(root: &Path, relative: &str) -> Option<Vec<u8>> {
    let authority = open_read_root::<DestinationRepositoryRoot>(root).expect("open read root");
    let relative = RelativePath::parse(relative).expect("valid relative path");
    let target = match authority.resolve_read(&relative, ObjectClass::RegularFile) {
        Ok(target) => target,
        Err(crate::platform::PathError::NotFound { .. }) => return None,
        Err(error) => panic!("unexpected read failure: {error}"),
    };
    let mut file = target.try_clone_file().expect("clone read handle");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).expect("read target");
    Some(bytes)
}

#[test]
fn equal_bytes_are_a_true_noop_without_any_effect() {
    let (_fixture, root) = fixture_root();
    let target = root.join("target.txt");
    fs::write(&target, b"authoritative bytes").expect("write target");
    let before = fs::metadata(&target)
        .expect("metadata")
        .modified()
        .expect("mtime");

    let current = read_current(&root, "target.txt").expect("target exists");
    let outcome = compare("op-1", "target.txt", Some(&current), b"authoritative bytes")
        .expect("compare must succeed");
    assert_eq!(outcome, CompareOutcome::Unchanged);

    let after = fs::metadata(&target)
        .expect("metadata")
        .modified()
        .expect("mtime");
    assert_eq!(before, after, "mtime must not change on a no-op");
    assert_eq!(fs::read(&target).expect("content"), b"authoritative bytes");
    let entries: Vec<_> = fs::read_dir(&root).expect("root").collect();
    assert_eq!(entries.len(), 1, "no temp or replacement artifacts");
}

#[test]
fn differing_bytes_return_a_prepared_replacement() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("target.txt"), b"old bytes").expect("write target");
    let current = read_current(&root, "target.txt").expect("target exists");
    let outcome =
        compare("op-1", "target.txt", Some(&current), b"new bytes").expect("compare must succeed");
    let CompareOutcome::Replacement(plan) = outcome else {
        panic!("expected replacement");
    };
    assert_eq!(plan.operation_id(), "op-1");
    assert_eq!(plan.target(), Path::new("target.txt"));
    assert_eq!(plan.parents(), &ParentDirectories::Existing);
    assert_eq!(
        fs::read(root.join("target.txt")).expect("content"),
        b"old bytes"
    );
}

#[test]
fn comparison_is_exact_and_never_normalizes() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("lines.txt"), "line one\nline two\n").expect("write LF target");
    let current = read_current(&root, "lines.txt").expect("target exists");
    for authoritative in [
        "line one\r\nline two\r\n".as_bytes(),
        "\u{feff}line one\nline two\n".as_bytes(),
        "line one\nline two \n".as_bytes(),
    ] {
        let outcome = compare("op-1", "lines.txt", Some(&current), authoritative)
            .expect("compare must succeed");
        assert!(
            matches!(outcome, CompareOutcome::Replacement(_)),
            "exact bytes must not be normalized"
        );
    }
    let outcome = compare(
        "op-1",
        "lines.txt",
        Some(&current),
        "line one\nline two\n".as_bytes(),
    )
    .expect("compare must succeed");
    assert_eq!(outcome, CompareOutcome::Unchanged);
}

#[test]
fn absent_target_is_a_prepared_create_replacement() {
    let (_fixture, root) = fixture_root();
    assert!(read_current(&root, "nested/deep/target.txt").is_none());
    let outcome = compare("op-2", "nested/deep/target.txt", None, b"new content")
        .expect("compare must succeed");
    let CompareOutcome::Replacement(plan) = outcome else {
        panic!("expected replacement");
    };
    assert_eq!(plan.target(), Path::new("nested/deep/target.txt"));
    let ParentDirectories::Created(parents) = plan.parents() else {
        panic!("expected created parents");
    };
    assert_eq!(
        parents,
        &[PathBuf::from("nested"), PathBuf::from("nested/deep")]
    );
    assert!(
        !root.join("nested").exists(),
        "the plan must not create anything"
    );
}

#[test]
fn read_failures_happen_before_compare_and_preserve_the_target() {
    let (_fixture, root) = fixture_root();
    fs::create_dir_all(root.join("adir")).expect("create directory target");
    // The caller-side platform read rejects a non-regular target; the target
    // is preserved and compare is never reached.
    let authority = open_read_root::<DestinationRepositoryRoot>(&root).expect("open read root");
    let relative = RelativePath::parse("adir").expect("valid relative path");
    assert!(
        authority
            .resolve_read(&relative, ObjectClass::RegularFile)
            .is_err()
    );
    assert!(root.join("adir").is_dir(), "the target must be preserved");
}

#[test]
fn hostile_paths_are_rejected_by_the_plan_contract() {
    let (_fixture, _root) = fixture_root();
    let error = compare("op-1", "../escape", Some(b"current"), b"authoritative")
        .expect_err("hostile target must fail");
    assert!(matches!(error, CompareError::Plan(_)), "{error:?}");
    let error = compare("op-1", "/absolute", Some(b"current"), b"authoritative")
        .expect_err("absolute target must fail");
    assert!(matches!(error, CompareError::Plan(_)), "{error:?}");
}

#[test]
fn compare_never_touches_git() {
    let (_fixture, root) = fixture_root();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git starts");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.name", "Compare"]);
    git(&["config", "user.email", "compare@example.test"]);
    fs::write(root.join("tracked.txt"), b"same").expect("write tracked");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    let before = git_status(&root);

    let current = read_current(&root, "tracked.txt").expect("target exists");
    compare("op-1", "tracked.txt", Some(&current), b"same").expect("compare");
    let after = git_status(&root);
    assert_eq!(
        before, after,
        "a no-op must not produce Git-visible mutation"
    );

    compare("op-1", "tracked.txt", Some(&current), b"different").expect("compare");
    assert_eq!(
        git_status(&root),
        after,
        "preparing a plan must not touch Git"
    );
}

fn git_status(root: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .expect("git status");
    String::from_utf8(output.stdout).expect("status is UTF-8")
}
