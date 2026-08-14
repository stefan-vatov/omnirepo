//! Focused proof for the frozen remote publication target.

#![allow(dead_code, unused_imports)]

use super::{PublicationPosture, RemoteTargetError, freeze_remote_target, sanitize_transport};
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::RefName;
use std::{fs, path::Path, process::Command};

fn fixture_repo() -> (
    tempfile::TempDir,
    AuthorityRoot<GitWorkingDirectoryRoot, ReadOnly>,
) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("remote-target-")
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
    // A bare upstream to push against (never contacted: the upstream ref
    // is created locally so @{u} resolves without any network).
    let upstream = fixture.path().join("upstream.git");
    git(&[
        "init",
        "--quiet",
        "--bare",
        upstream.to_str().expect("path"),
    ]);
    let remote_url = "ssh://git@example.test/upstream.git".to_owned();
    git(&["remote", "add", "origin", &remote_url]);
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    let local_oid = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-parse", "HEAD"])
        .current_dir(&root)
        .output()
        .expect("git");
    let local_oid = String::from_utf8(local_oid.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    git(&["update-ref", "refs/remotes/origin/main", &local_oid]);
    git(&["branch", "--set-upstream-to=origin/main", "main"]);
    let authority =
        AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&root).expect("authority root");
    (fixture, authority)
}

fn ref_name(value: &str) -> RefName {
    RefName::new(value).expect("ref name")
}

#[test]
fn frozen_target_records_the_canonical_tuple() {
    let (_fixture, authority) = fixture_repo();
    let (target, posture) = freeze_remote_target(&authority).expect("freeze");
    assert_eq!(target.remote, "origin");
    assert_eq!(target.reference, ref_name("refs/heads/main"));
    assert!(!target.oid.as_str().is_empty());
    assert!(
        matches!(posture, PublicationPosture::InSync { .. }),
        "{posture:?}"
    );
}

#[test]
fn ahead_behind_and_diverged_postures_are_exact() {
    let (_fixture, authority) = fixture_repo();
    let working = authority.display_path().as_path();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(working)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    let rev_parse = |rev: &str| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["rev-parse", rev])
            .current_dir(working)
            .output()
            .expect("git");
        String::from_utf8(output.stdout)
            .expect("stdout")
            .trim()
            .to_owned()
    };
    // One local commit ahead: ahead=1.
    fs::write(working.join("managed.txt"), "v2\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "ahead"]);
    let (_target, posture) = freeze_remote_target(&authority).expect("freeze");
    match posture {
        PublicationPosture::Ahead { ahead, .. } => assert_eq!(ahead, 1),
        other => panic!("expected ahead, got {other:?}"),
    }

    // The remote advances one commit (a commit-tree sibling of HEAD); the
    // local branch stays: behind=1.
    let head_tree = rev_parse("HEAD^{tree}");
    let remote_commit = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args([
            "commit-tree",
            &head_tree,
            "-p",
            &rev_parse("HEAD"),
            "-m",
            "remote-only",
        ])
        .current_dir(working)
        .output()
        .expect("git");
    let remote_oid = String::from_utf8(remote_commit.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    git(&["update-ref", "refs/remotes/origin/main", &remote_oid]);
    let (_target, posture) = freeze_remote_target(&authority).expect("freeze");
    match posture {
        PublicationPosture::Behind { behind, .. } => assert_eq!(behind, 1),
        other => panic!("expected behind, got {other:?}"),
    }

    // Both sides advance: divergent.
    fs::write(working.join("managed.txt"), "v3\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "diverged"]);
    let (_target, posture) = freeze_remote_target(&authority).expect("freeze");
    match posture {
        PublicationPosture::Diverged { ahead, behind, .. } => {
            assert_eq!(ahead, 1, "{ahead}");
            assert_eq!(behind, 1, "{behind}");
        }
        other => panic!("expected diverged, got {other:?}"),
    }
}

#[test]
fn detached_and_no_upstream_fail_typed() {
    let (_fixture, authority) = fixture_repo();
    let working = authority.display_path().as_path();
    let _ = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["checkout", "--quiet", "--detach"])
        .current_dir(working)
        .status()
        .expect("git");
    let error = freeze_remote_target(&authority).expect_err("detached");
    assert!(
        matches!(error, RemoteTargetError::Detached { .. }),
        "{error}"
    );

    // Reattach and drop the upstream.
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(working)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["checkout", "--quiet", "main"]);
    let _ = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["branch", "--unset-upstream"])
        .current_dir(working)
        .status()
        .expect("git");
    let error = freeze_remote_target(&authority).expect_err("no upstream");
    assert!(
        matches!(error, RemoteTargetError::NoUpstream { .. }),
        "{error}"
    );
}

#[test]
fn unsanitized_transports_fail_before_contact() {
    let (_fixture, authority) = fixture_repo();
    let working = authority.display_path().as_path();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(working)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&[
        "remote",
        "set-url",
        "origin",
        "http://example.test/upstream.git",
    ]);
    let error = freeze_remote_target(&authority).expect_err("plaintext http");
    assert!(
        matches!(error, RemoteTargetError::TransportUnsanitized { .. }),
        "{error}"
    );
    git(&[
        "remote",
        "set-url",
        "origin",
        "ssh://user:password@example.test/upstream.git",
    ]);
    let error = freeze_remote_target(&authority).expect_err("embedded credentials");
    assert!(
        matches!(error, RemoteTargetError::TransportUnsanitized { .. }),
        "{error}"
    );
    // The pure sanitizer rejects non-https/ssh schemes too.
    assert!(sanitize_transport("file:///tmp/repo", "origin").is_err());
    assert!(sanitize_transport("git://example.test/repo", "origin").is_err());
    assert!(sanitize_transport("https://example.test/repo", "origin").is_ok());
    assert!(sanitize_transport("ssh://git@example.test/repo", "origin").is_ok());
}
