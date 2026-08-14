//! Focused proof for the owner-selected exact-SHA release trigger.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_trigger::{
    TriggerError, TriggerVerification, verify_exact_sha_trigger,
};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-trigger-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(root: &Path) {
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
    git(&["config", "user.name", "Trigger"]);
    git(&["config", "user.email", "trigger@example.test"]);
    fs::write(root.join("file.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
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
fn the_exact_sha_trigger_verifies_when_tag_commit_and_head_match() {
    let fixture = fixture_base();
    let root = fixture.path().join("repo");
    git_repo(&root);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let tag = git_text(&root, &["rev-parse", "HEAD"]);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["tag", "v0.9.0", &tag]);
    let verification = verify_exact_sha_trigger(&root, "v0.9.0", &tag, &head).expect("trigger");
    assert!(verification.verified);
    assert_eq!(verification.commit, head);
}

#[test]
fn a_head_mismatch_refuses_the_trigger() {
    let fixture = fixture_base();
    let root = fixture.path().join("repo");
    git_repo(&root);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["tag", "v0.9.0", &head]);
    // A later commit moves the head away from the tag's commit.
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    fs::write(root.join("file.txt"), "v2\n").expect("change");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "second"]);
    let head_now = git_text(&root, &["rev-parse", "HEAD"]);
    assert_ne!(head_now, head, "the head moved");
    // The exact-SHA input pins the tag's commit; the current head has
    // moved away, so the trigger is refused.
    let error = verify_exact_sha_trigger(&root, "v0.9.0", &head, &head).expect_err("mismatch");
    assert!(
        matches!(error, TriggerError::HeadMismatch { .. }),
        "{error}"
    );
}

#[test]
fn a_missing_tag_is_refused() {
    let fixture = fixture_base();
    let root = fixture.path().join("repo");
    git_repo(&root);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["tag", "v0.9.0", &head]);
    let error = verify_exact_sha_trigger(
        &root,
        "v0.9.0",
        "0000000000000000000000000000000000000000",
        &head,
    )
    .expect_err("missing tag");
    assert!(matches!(error, TriggerError::TagMismatch { .. }), "{error}");
}

#[test]
fn the_release_workflow_uses_only_the_tag_trigger_with_the_sha_input() {
    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml"),
    )
    .expect("release workflow");
    // The owner-selected trigger: version tags only, with the exact SHA
    // as a required input; no schedule or other trigger path.
    assert!(workflow.contains("push:"));
    assert!(workflow.contains("tags:"));
    assert!(workflow.contains("v*"));
    assert!(!workflow.contains("schedule:"));
    assert!(workflow.contains("workflow_dispatch"));
    assert!(workflow.contains("commit"));
    assert!(workflow.contains("required: true"));
}
