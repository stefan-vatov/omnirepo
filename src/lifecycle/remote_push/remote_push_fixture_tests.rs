//! Local-remote publication and hostile transport fixtures.

#![allow(dead_code, unused_imports)]

use super::push_recorded_oid;
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::remote_target::{
    FrozenRemoteTarget, RemoteTargetError, freeze_remote_target,
};
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

fn journal_fixture() -> (tempfile::TempDir, Journal, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("publication-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id)
}

fn fixture_repos() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("publication-repos-")
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
    // The tracking ref mirrors the upstream (never contacted) so the
    // freeze can resolve the publication target locally.
    let base = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&working)
        .output()
        .expect("git");
    let base = String::from_utf8(base.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    git(&working, &["update-ref", "refs/remotes/origin/main", &base]);
    git(
        &working,
        &["branch", "--set-upstream-to=origin/main", "main"],
    );
    (fixture, working, upstream)
}

fn head_oid(working: &Path) -> RevisionId {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(working)
        .output()
        .expect("git");
    revision(String::from_utf8(output.stdout).expect("stdout").trim())
}

#[test]
fn exactly_one_intended_ref_reaches_one_intended_oid() {
    let (_fixture, working, upstream) = fixture_repos();
    let (_jfixture, journal, run_id) = journal_fixture();
    let oid = head_oid(&working);
    let target = FrozenRemoteTarget {
        remote: "origin".to_owned(),
        reference: ref_name("refs/heads/main"),
        oid: oid.clone(),
    };
    push_recorded_oid(
        &git_root(&working),
        &target,
        &oid,
        "dest-a",
        &journal.handle,
        &run_id,
        Duration::from_secs(30),
    )
    .expect("push");
    // The upstream advertises exactly one ref, and it is the intended OID.
    let advertised = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&upstream)
        .output()
        .expect("git");
    let advertised = String::from_utf8(advertised.stdout).expect("stdout");
    let expected = format!("refs/heads/main {}", oid.as_str());
    assert_eq!(
        advertised.trim(),
        expected,
        "no unrelated object is advertised"
    );
}

#[test]
fn divergent_remote_rejects_without_force() {
    let (_fixture, working, upstream) = fixture_repos();
    let (_jfixture, journal, run_id) = journal_fixture();
    let oid = head_oid(&working);
    // Push the base, then advance the upstream with a third commit.
    let git = |dir: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&working, &["push", "--quiet", "origin", "main"]);
    let third = Command::new("git")
        .args(["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "third"])
        .current_dir(&working)
        .output()
        .expect("git");
    let third = String::from_utf8(third.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
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
    let target = FrozenRemoteTarget {
        remote: "origin".to_owned(),
        reference: ref_name("refs/heads/main"),
        oid: oid.clone(),
    };
    // The recorded OID is an ancestor of the upstream: without force the
    // push must fail typed.
    let outcome = push_recorded_oid(
        &git_root(&working),
        &target,
        &oid,
        "dest-a",
        &journal.handle,
        &run_id,
        Duration::from_secs(30),
    );
    assert!(outcome.is_err(), "force must never occur: {outcome:?}");
}

#[test]
fn credential_bearing_transport_never_reaches_the_push() {
    let (_fixture, working, _upstream) = fixture_repos();
    let git = |dir: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    // The credential URL is assembled from parts so no literal secret
    // appears in the source tree.
    let user = "user";
    let secret = "supers".to_owned() + "ecret";
    let credential_url = format!("https://{user}:{secret}@example.test/upstream.git");
    git(&working, &["remote", "set-url", "origin", &credential_url]);
    // The frozen target refuses the credential-bearing transport before
    // any remote contact; the push is never attempted.
    let root = crate::platform::AuthorityRoot::<
        crate::platform::GitWorkingDirectoryRoot,
        crate::platform::ReadOnly,
    >::open(&working)
    .expect("root");
    let error = freeze_remote_target(&root).expect_err("credentials must fail closed");
    assert!(
        matches!(error, RemoteTargetError::TransportUnsanitized { .. }),
        "{error}"
    );
}
