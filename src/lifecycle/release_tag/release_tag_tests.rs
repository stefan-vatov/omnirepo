//! Focused proof for creating and validating the canonical release tag
//! and identity.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_tag::{
    TagError, TagOutcome, TagValidation, create_canonical_tag, validate_canonical_tag,
};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-tag-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(root: &Path) -> (String, String) {
    fs::create_dir_all(root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Tag"]);
    git(&["config", "user.email", "tag@example.test"]);
    fs::write(root.join("file.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    let head = git_text(root, &["rev-parse", "HEAD"]);
    (root.to_path_buf().display().to_string(), head)
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {:?}", output);
    String::from_utf8(output.stdout)
        .expect("stdout")
        .trim()
        .to_owned()
}

#[test]
fn the_canonical_tag_is_created_annotated_at_the_exact_commit() {
    let fixture = fixture_base();
    let (root, commit) = git_repo(&fixture.path().join("repo"));
    let outcome = create_canonical_tag(&root, "0.9.0", &commit).expect("create");
    assert!(matches!(outcome, TagOutcome::Created { .. }));
    // The tag is annotated and points at the exact commit.
    let validation = validate_canonical_tag(&root, "0.9.0").expect("validate");
    assert!(validation.annotated);
    assert_eq!(validation.commit, commit);
}

#[test]
fn an_existing_tag_at_the_same_commit_is_idempotent() {
    let fixture = fixture_base();
    let (root, commit) = git_repo(&fixture.path().join("repo"));
    create_canonical_tag(&root, "0.9.0", &commit).expect("first");
    let second = create_canonical_tag(&root, "0.9.0", &commit).expect("second");
    assert!(matches!(second, TagOutcome::Existing { .. }), "{second:?}");
}

#[test]
fn an_existing_tag_at_a_different_commit_is_refused() {
    let fixture = fixture_base();
    let (root, commit) = git_repo(&fixture.path().join("repo"));
    create_canonical_tag(&root, "0.9.0", &commit).expect("first");
    // A later commit: the tag already exists at the old commit.
    let root_path = Path::new(&root);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(root_path)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    fs::write(root_path.join("file.txt"), "v2\n").expect("change");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "second"]);
    let new_commit = git_text(root_path, &["rev-parse", "HEAD"]);
    let outcome = create_canonical_tag(&root, "0.9.0", &new_commit).expect("refused");
    assert!(matches!(outcome, TagOutcome::Refused { .. }), "{outcome:?}");
}

#[test]
fn a_missing_or_non_canonical_tag_fails_typed() {
    let fixture = fixture_base();
    let (root, commit) = git_repo(&fixture.path().join("repo"));
    assert!(matches!(
        validate_canonical_tag(&root, "0.9.0"),
        Err(TagError::Missing { .. })
    ));
    create_canonical_tag(&root, "0.9.0", &commit).expect("create");
    assert!(matches!(
        validate_canonical_tag(&root, "not-a-version"),
        Err(TagError::NonCanonicalName { .. })
    ));
}
