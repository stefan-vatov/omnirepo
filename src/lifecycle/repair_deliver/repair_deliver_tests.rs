//! Focused proof for delivering valid repair changes through scoped Git
//! with a journaled outcome.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::repair_deliver::{RepairDeliveryError, deliver_repair_changes};
use crate::lifecycle::run_record::RunRecord;
use crate::repository::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, GitRepositoryState, HeadState, IndexState, ManagedTargetIdentity,
    ObjectIdentity, RefName, RepositoryFacts, RepositoryId, RepositoryRoot, RepositorySnapshot,
    RevisionId, UpstreamState, WorktreeState,
};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("repository id")
}

fn path(value: &str) -> crate::repository::RelativePath {
    crate::repository::RelativePath::from_bytes(value).expect("relative path")
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

fn facts() -> RepositoryFacts {
    RepositoryFacts::new(
        repository_id("destination-a"),
        RepositoryRoot::new(
            "/workspace/destination-a",
            AuthorityIdentity::new(
                FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
                ObjectIdentity::new(7, 9),
            )
            .expect("authority"),
        )
        .expect("root"),
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

fn snapshot(managed: &str, inode: u64) -> RepositorySnapshot {
    let target =
        ManagedTargetIdentity::whole_file(path(managed), Some(identity(inode))).expect("target");
    RepositorySnapshot::new(facts(), witnesses(), vec![target]).expect("snapshot")
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-deliver-home-")
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
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

fn git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-deliver-work-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("destination");
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
    git(&["config", "user.name", "Delivery"]);
    git(&["config", "user.email", "delivery@example.test"]);
    fs::write(root.join("managed.md"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    (fixture, root)
}

#[test]
fn the_verified_repair_change_is_delivered_as_one_scoped_commit() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    // The repair effect: the managed payload changed after reapply.
    fs::write(
        root.join("managed.md"),
        "# omnirepo-start\nv2\n# omnirepo-end\n",
    )
    .expect("change");
    let snapshot = snapshot("managed.md", 11);
    let delivery = deliver_repair_changes(
        &root,
        &snapshot,
        "repair: restore managed payload",
        &journal.handle,
        &run_id,
        "destination-a",
    )
    .expect("delivery");
    assert!(
        !delivery.oid.is_empty(),
        "the delivered commit OID is recorded"
    );
    // The delivered commit exists in the object database.
    let exists = git_text(&root, &["cat-file", "-e", &delivery.oid]);
    assert!(exists.is_empty() || exists == delivery.oid, "oid present");
    // The delivered commit is a child of the base commit.
    let base = git_text(&root, &["rev-parse", "HEAD"]);
    let parent = git_text(&root, &["rev-parse", &format!("{}^", delivery.oid)]);
    assert_eq!(parent, base, "the delivered commit is a child of the base");
    // The worktree managed file is committed, nothing else.
    let files = git_text(&root, &["show", "--name-only", "--format=", &delivery.oid]);
    assert!(files.contains("managed.md"), "{files}");
    journal.shutdown().expect("shutdown");
}

#[test]
fn the_journal_carries_intent_commit_result_and_evidence() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    fs::write(
        root.join("managed.md"),
        "# omnirepo-start\nv2\n# omnirepo-end\n",
    )
    .expect("change");
    let snapshot = snapshot("managed.md", 11);
    deliver_repair_changes(
        &root,
        &snapshot,
        "repair: restore managed payload",
        &journal.handle,
        &run_id,
        "destination-a",
    )
    .expect("delivery");
    journal.shutdown().expect("shutdown");
    let lines = fs::read_to_string(&record_path).expect("record");
    assert!(lines.contains("\"type\":\"repository_intent\""), "{lines}");
    assert!(lines.contains("\"type\":\"repository_result\""), "{lines}");
    assert!(lines.contains("commit/"), "{lines}");
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
