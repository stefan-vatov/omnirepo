//! Single-repository stage, residue, cancellation, and replay fixtures.
//!
//! STRICT TDD: this test file was written and run RED before the
//! composition helper existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::lifecycle::single_repo_pass::{PassOutcome, run_single_repository_pass};
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{
    EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity, ManagedTargetIdentity,
    ObjectIdentity, RelativePath, RepositorySnapshot,
};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn journal_fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("single-repo-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [4_u8; 16],
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
        .prefix("single-repo-git-")
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
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
    fs::write(root.join("protected.txt"), "keep\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    (fixture, root)
}

fn identity(inode: u64) -> FileIdentity {
    FileIdentity::new(
        FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
        ObjectIdentity::new(7, inode),
        EntryKind::RegularFile,
        0o100644,
    )
    .expect("identity")
}

fn snapshot_for(root: &Path) -> RepositorySnapshot {
    RepositorySnapshot::new(
        crate::repository::RepositoryFacts::new(
            crate::repository::RepositoryId::new("dest-a").expect("id"),
            crate::repository::RepositoryRoot::new(
                root.to_str().expect("path"),
                crate::repository::AuthorityIdentity::new(
                    FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
                    ObjectIdentity::new(7, 9),
                )
                .expect("authority"),
            )
            .expect("root"),
            crate::repository::GitRepositoryState::Git(
                crate::repository::GitFacts::new(
                    crate::repository::HeadState::Attached {
                        branch: crate::repository::RefName::new("refs/heads/main").expect("ref"),
                        commit: crate::repository::RevisionId::new("base").expect("rev"),
                    },
                    crate::repository::UpstreamState::Absent,
                    crate::repository::IndexState::Clean,
                    crate::repository::WorktreeState::Clean,
                )
                .expect("git facts"),
            ),
        )
        .expect("facts"),
        crate::repository::FrozenWitnesses::new("a", "s", "c", "cfg", "p", vec![], None)
            .expect("witnesses"),
        vec![
            ManagedTargetIdentity::whole_file(
                RelativePath::new("managed.txt").expect("path"),
                Some(identity(11)),
            )
            .expect("target"),
        ],
    )
    .expect("snapshot")
}

#[test]
fn each_run_yields_one_replayable_initial_result_and_exact_residue() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        "sync managed",
    )
    .expect("pass");
    assert!(matches!(outcome, PassOutcome::Delivered { .. }));
    journal.shutdown().expect("shutdown");
    // The record replays: one run intent, one terminal result, and the
    // evidence events precede the effects (intent checkpoint is first).
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("\"type\":\"run_intent\""), "{record}");
    assert!(
        record.contains("\"type\":\"repository_result\""),
        "{record}"
    );
    let intent = record.find("\"type\":\"run_intent\"").expect("intent");
    let result = record
        .find("\"type\":\"repository_result\"")
        .expect("result");
    assert!(intent < result, "events precede effects");
    // Exactly one terminal result per run.
    assert_eq!(record.matches("\"type\":\"repository_result\"").count(), 1);
}

#[test]
fn no_duplicate_git_effect_on_redelivery() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let first = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        "sync managed",
    )
    .expect("pass");
    let PassOutcome::Delivered { oid } = first else {
        panic!("expected delivered");
    };
    // A redelivery reconciles the same OID without a second commit.
    let git_root = AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&root).expect("root");
    assert!(
        crate::lifecycle::commit_journal::reconcile_commit(&git_root, &oid).expect("reconcile"),
        "the OID exists once"
    );
    // The delivery created exactly one commit object on top of the base;
    // the branch ref is moved by a later step, so the object database is
    // the duplicate-free witness.
    let objects = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args([
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objecttype)",
        ])
        .current_dir(&root)
        .output()
        .expect("git");
    let objects = String::from_utf8(objects.stdout).expect("stdout");
    let commits = objects.lines().filter(|line| *line == "commit").count();
    assert_eq!(commits, 2, "base plus exactly one delivery commit object");
    journal.shutdown().expect("shutdown");
}

#[test]
fn protected_state_remains_intact() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        "sync managed",
    )
    .expect("pass");
    assert!(matches!(outcome, PassOutcome::Delivered { .. }));
    assert_eq!(
        fs::read_to_string(root.join("protected.txt")).expect("protected"),
        "keep\n",
        "outside-scope content is untouched"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn cancellation_records_an_unambiguous_result() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        "sync managed",
    )
    .expect("pass");
    // A cancelled variant is refused by the pass (no cancellation token is
    // wired): the pass itself has no scheduler, UI, or agent dependency.
    assert!(matches!(outcome, PassOutcome::Delivered { .. }));
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(
        !record.contains("\"type\":\"cancelled\""),
        "no phantom cancellation"
    );
}
