//! Focused proof for push ambiguity reconciliation.

#![allow(dead_code, unused_imports)]

use super::{ReconcileError, ReconcileOutcome, reconcile_push};
use crate::lifecycle::remote_target::FrozenRemoteTarget;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{RefName, RevisionId};
use std::{fs, path::Path, process::Command};

fn head_oid(working: &std::path::Path) -> RevisionId {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-parse", "HEAD"])
        .current_dir(working)
        .output()
        .expect("git");
    revision(String::from_utf8(output.stdout).expect("stdout").trim())
}

fn git_root(working: &std::path::Path) -> AuthorityRoot<GitWorkingDirectoryRoot, ReadOnly> {
    AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(working).expect("git root")
}

fn ref_name(value: &str) -> RefName {
    RefName::new(value).expect("ref name")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

/// A working repo with a bare upstream; the remote ref is set with
/// update-ref (objects pushed into the upstream first), so ls-remote reads
/// a real state without any network.
fn fixture_repos() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("push-reconcile-")
        .tempdir_in(&base)
        .expect("fixture");
    let working = fixture.path().join("working");
    let upstream = fixture.path().join("upstream.git");
    fs::create_dir_all(&working).expect("working");
    let git = |dir: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&working, &["init", "--quiet", "-b", "main"]);
    git(&working, &["config", "user.name", "Commit"]);
    git(&working, &["config", "user.email", "commit@example.test"]);
    git(
        &working,
        &[
            "init",
            "--quiet",
            "--bare",
            upstream.to_str().expect("path"),
        ],
    );
    git(
        &working,
        &["remote", "add", "origin", upstream.to_str().expect("path")],
    );
    fs::write(working.join("managed.txt"), "v1\n").expect("file");
    git(&working, &["add", "."]);
    git(&working, &["commit", "--quiet", "--message", "base"]);
    fs::write(working.join("managed.txt"), "v2\n").expect("file");
    git(&working, &["add", "."]);
    git(&working, &["commit", "--quiet", "--message", "recorded"]);
    // Push the recorded commit so the upstream has real objects and its
    // ref equals the recorded OID.
    git(&working, &["push", "--quiet", "origin", "main"]);
    let oid = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-parse", "HEAD"])
        .current_dir(&working)
        .output()
        .expect("git");
    let _oid = String::from_utf8(oid.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    (fixture, working, upstream)
}

fn target() -> FrozenRemoteTarget {
    FrozenRemoteTarget {
        remote: "origin".to_owned(),
        reference: ref_name("refs/heads/main"),
        oid: revision("0000000000000000000000000000000000000000"),
    }
}

#[test]
fn remote_already_new_records_success_without_repush() {
    let (_fixture, working, _upstream) = fixture_repos();
    let recorded = head_oid(&working);
    let outcome = reconcile_push(
        &git_root(&working),
        &target(),
        &recorded,
        &revision("pre-push-placeholder"),
    )
    .expect("reconcile");
    // The remote ref IS the recorded OID (the fixture pushed it): accepted,
    // nothing to repush.
    assert_eq!(
        outcome,
        ReconcileOutcome::Accepted {
            recorded: recorded.clone()
        }
    );
}

#[test]
fn remote_old_allows_retry_within_policy() {
    let (_fixture, working, upstream) = fixture_repos();
    let recorded = head_oid(&working);
    // The pre-push OID differs from the remote: the recorded commit was
    // never accepted (e.g. a disconnect after accept).  Rewind the remote
    // ref to the parent so it is neither the recorded nor a third OID.
    let parent = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-parse", "HEAD~1"])
        .current_dir(&working)
        .output()
        .expect("git");
    let parent = revision(String::from_utf8(parent.stdout).expect("stdout").trim());
    let rewind = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["update-ref", "refs/heads/main", parent.as_str()])
        .current_dir(&upstream)
        .output()
        .expect("git");
    assert!(
        rewind.status.success(),
        "rewind failed: {:?}",
        String::from_utf8_lossy(&rewind.stderr)
    );
    let outcome =
        reconcile_push(&git_root(&working), &target(), &recorded, &parent).expect("reconcile");
    assert_eq!(
        outcome,
        ReconcileOutcome::RetryAllowed {
            recorded: recorded.clone(),
            remote: parent.clone()
        }
    );
}

#[test]
fn third_oid_is_a_conflict_without_force() {
    let (_fixture, working, upstream) = fixture_repos();
    let recorded = head_oid(&working);
    // A third OID: a remote-only commit on top of the recorded one.
    let third = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "third"])
        .current_dir(&working)
        .output()
        .expect("git");
    let third = String::from_utf8(third.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    let git = |dir: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(
        &working,
        &[
            "push",
            "--quiet",
            "origin",
            &format!("{third}:refs/heads/scratch"),
        ],
    );
    git(&upstream, &["update-ref", "refs/heads/main", &third]);
    let outcome = reconcile_push(
        &git_root(&working),
        &target(),
        &recorded,
        &revision("pre-push-placeholder"),
    )
    .expect("reconcile");
    assert!(
        matches!(outcome, ReconcileOutcome::Conflict { .. }),
        "{outcome:?}"
    );
}

trait StdoutPipe {
    fn pipe(self, f: fn(Vec<u8>) -> Result<String, std::string::FromUtf8Error>) -> String;
}
impl StdoutPipe for Vec<u8> {
    fn pipe(self, f: fn(Vec<u8>) -> Result<String, std::string::FromUtf8Error>) -> String {
        f(self).expect("stdout").trim().to_owned()
    }
}
