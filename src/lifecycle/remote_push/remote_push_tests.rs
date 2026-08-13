//! Focused proof for the exact-OID push.

#![allow(dead_code, unused_imports)]

use super::{PushError, push_recorded_oid};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::remote_target::FrozenRemoteTarget;
use crate::lifecycle::run_record::RunRecord;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{RefName, RevisionId};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn git_root(working: &std::path::Path) -> AuthorityRoot<GitWorkingDirectoryRoot, ReadOnly> {
    AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(working).expect("git root")
}

fn ref_name(value: &str) -> RefName {
    RefName::new(value).expect("ref name")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("push-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [9_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

/// A working repo with a bare upstream; both refs live locally, so the push
/// never needs a real network.
fn fixture_repos() -> (tempfile::TempDir, PathBufFixture) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("push-repos-")
        .tempdir_in(&base)
        .expect("fixture");
    let working = fixture.path().join("working");
    let upstream = fixture.path().join("upstream.git");
    fs::create_dir_all(&working).expect("working");
    let git = |dir: &Path, args: &[&str]| {
        let output = Command::new("git")
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
    // The remote tracking ref is set to an OLD commit so the push advances
    // exactly the selected ref.
    let base_oid = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&working)
        .output()
        .expect("git");
    let base_oid = String::from_utf8(base_oid.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    git(
        &working,
        &["update-ref", "refs/remotes/origin/main", &base_oid],
    );
    git(
        &working,
        &["branch", "--set-upstream-to=origin/main", "main"],
    );
    fs::write(working.join("managed.txt"), "v2\n").expect("file");
    git(&working, &["add", "."]);
    git(&working, &["commit", "--quiet", "--message", "recorded"]);
    (fixture, PathBufFixture { working, upstream })
}

struct PathBufFixture {
    working: std::path::PathBuf,
    upstream: std::path::PathBuf,
}

#[test]
fn push_sends_only_the_recorded_oid_to_the_selected_ref() {
    let (_fixture, repos) = fixture_repos();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    let oid = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repos.working)
        .output()
        .expect("git");
    let oid = revision(String::from_utf8(oid.stdout).expect("stdout").trim());
    let target = FrozenRemoteTarget {
        remote: "origin".to_owned(),
        reference: ref_name("refs/heads/main"),
        oid: oid.clone(),
    };
    let outcome = push_recorded_oid(
        &git_root(&repos.working),
        &target,
        &oid,
        "dest-a",
        &journal.handle,
        &run_id,
        Duration::from_secs(30),
    )
    .expect("push");
    assert!(outcome.pushed);
    journal.shutdown().expect("shutdown");
    // The upstream ref is exactly the recorded OID.
    let upstream_ref = Command::new("git")
        .args(["rev-parse", "refs/heads/main"])
        .current_dir(&repos.upstream)
        .output()
        .expect("git");
    assert_eq!(
        String::from_utf8(upstream_ref.stdout)
            .expect("stdout")
            .trim(),
        oid.as_str()
    );
    // No tags and no incidental refs were sent: the bare upstream carries
    // exactly one ref.
    let refs = Command::new("git")
        .args(["for-each-ref", "--format=%(refname)"])
        .current_dir(&repos.upstream)
        .output()
        .expect("git");
    let refs = String::from_utf8(refs.stdout).expect("stdout");
    assert_eq!(refs.trim(), "refs/heads/main");
}

#[test]
fn remote_rejection_is_typed_and_intent_is_journaled() {
    let (_fixture, repos) = fixture_repos();
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    let oid = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repos.working)
        .output()
        .expect("git");
    let oid = revision(String::from_utf8(oid.stdout).expect("stdout").trim());
    // The upstream already advanced elsewhere: first land the recorded
    // commit, then build a remote-only child ON TOP of it, then move the
    // upstream ref forward (all objects exist in both stores).
    let git = |dir: &std::path::Path, args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(
        &repos.working,
        &[
            "push",
            "--quiet",
            "origin",
            &format!("{}:refs/heads/main", oid.as_str()),
        ],
    );
    let remote_commit = Command::new("git")
        .args([
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            "HEAD",
            "-m",
            "remote-only",
        ])
        .current_dir(&repos.working)
        .output()
        .expect("git");
    let remote_oid = String::from_utf8(remote_commit.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    git(
        &repos.working,
        &[
            "push",
            "--quiet",
            "origin",
            &format!("{remote_oid}:refs/heads/scratch"),
        ],
    );
    git(
        &repos.upstream,
        &["update-ref", "refs/heads/main", &remote_oid],
    );
    let target = FrozenRemoteTarget {
        remote: "origin".to_owned(),
        reference: ref_name("refs/heads/main"),
        oid: oid.clone(),
    };
    let error = push_recorded_oid(
        &git_root(&repos.working),
        &target,
        &oid,
        "dest-a",
        &journal.handle,
        &run_id,
        Duration::from_secs(30),
    )
    .expect_err("rejected push");
    assert!(matches!(error, PushError::RemoteRejected { .. }), "{error}");
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("\"operation\":\"push\""), "{record}");
}
