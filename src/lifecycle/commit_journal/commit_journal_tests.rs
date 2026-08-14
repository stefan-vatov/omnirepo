//! Focused proof for journaled operation commits and reconciliation.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::repository::PlannedOperation;
use crate::repository::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, GitRepositoryState, HeadState, IndexState, ManagedTargetIdentity,
    ObjectIdentity, RefName, RelativePath, RepositoryFacts, RepositoryId, RepositoryRoot,
    RepositorySnapshot, RevisionId, UpstreamState, WorktreeState,
};
use std::{fs, path::Path, path::PathBuf, process::Command, time::Duration, time::SystemTime};

fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("repository id")
}
fn root(value: &str) -> RepositoryRoot {
    RepositoryRoot::new(value, authority(0)).expect("repository root")
}
fn path(value: &str) -> RelativePath {
    RelativePath::from_bytes(value).expect("relative path")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}
fn ref_name(value: &str) -> RefName {
    RefName::new(value).expect("ref name")
}
fn witness(value: &str) -> CheckWitness {
    CheckWitness::new(value).expect("check witness")
}
fn identity(inode: u64) -> FileIdentity {
    FileIdentity::new(
        FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
        ObjectIdentity::new(7, inode),
        EntryKind::RegularFile,
        0o100644,
    )
    .expect("file identity")
}
fn authority(inode: u64) -> AuthorityIdentity {
    AuthorityIdentity::new(
        FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
        ObjectIdentity::new(7, inode),
    )
    .expect("authority identity")
}
fn witnesses() -> FrozenWitnesses {
    FrozenWitnesses::new(
        "authority-1",
        "source-1",
        "catalog-1",
        "configuration-1",
        "plan-1",
        vec![witness("check-a")],
        Some(revision("base-1")),
    )
    .expect("witnesses")
}
fn facts() -> RepositoryFacts {
    RepositoryFacts::new(
        repository_id("destination-a"),
        root("/workspace/destination-a"),
        GitRepositoryState::Git(
            GitFacts::new(
                HeadState::Attached {
                    branch: ref_name("refs/heads/main"),
                    commit: revision("head-1"),
                },
                UpstreamState::Configured {
                    remote: "origin".into(),
                    reference: ref_name("refs/heads/main"),
                    commit: revision("remote-1"),
                },
                IndexState::Clean,
                WorktreeState::Clean,
            )
            .expect("git facts"),
        ),
    )
    .expect("facts")
}
fn baseline(path_value: &str, inode: u64) -> RepositorySnapshot {
    let target =
        ManagedTargetIdentity::whole_file(path(path_value), Some(identity(inode))).expect("target");
    RepositorySnapshot::new(facts(), witnesses(), vec![target]).expect("snapshot")
}

fn fixture_journal() -> (tempfile::TempDir, Journal, PathBuf, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("commit-journal-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [3_u8; 16],
    )
    .expect("record");
    let path = record.path().to_path_buf();
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, path, run_id)
}

fn fixture_repo_root() -> (tempfile::TempDir, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("commit-journal-repo-")
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
    (fixture, root)
}

fn write(root: &Path, relative: &str, content: &str) {
    fs::write(root.join(relative), content).expect("write");
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
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
fn journaled_commit_records_intent_result_and_exact_oid() {
    let (_jfixture, mut journal, record_path, run_id) = fixture_journal();
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    let base = git_text(&root, &["rev-parse", "HEAD"]);
    write(&root, "managed.txt", "v2\n");
    let snapshot = baseline("managed.txt", 11);
    let delta = crate::repository::build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let isolated = crate::repository::prepare_index(&root, &delta).expect("index");
    let git_root = crate::platform::AuthorityRoot::<
        crate::platform::GitWorkingDirectoryRoot,
        crate::platform::ReadOnly,
    >::open(&root)
    .expect("git root");
    let recorded = super::create_commit_journaled(
        &git_root,
        &isolated,
        Some(&base),
        "chore(omnirepo): sync managed content",
        &journal.handle,
        &run_id,
        "destination-a",
    )
    .expect("journaled commit");
    // The OID exists in the object database and reconciles; a bogus OID does
    // not.
    assert!(super::reconcile_commit(&git_root, &recorded.sha).expect("reconcile"));
    assert!(
        !super::reconcile_commit(&git_root, "0000000000000000000000000000000000000000")
            .expect("reconcile bogus")
    );
    journal
        .handle
        .submit(crate::lifecycle::event::JournalEvent::Terminal {
            checkpoint: 0,
            run_id: run_id.clone(),
            outcome: crate::lifecycle::event::Outcome::Success,
        })
        .expect("terminal");
    journal.shutdown().expect("shutdown");
    let content = fs::read_to_string(&record_path).expect("record");
    assert!(content.contains("\"operation\":\"commit\""), "{content}");
    assert!(content.contains(&recorded.sha), "{content}");
    assert!(content.contains("\"stage\":\"commit\""), "{content}");
}
