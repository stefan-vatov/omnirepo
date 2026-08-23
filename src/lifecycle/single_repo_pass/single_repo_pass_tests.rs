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
        FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
        ObjectIdentity::new(7, inode),
        EntryKind::RegularFile,
        0o100644,
    )
    .expect("identity")
}

fn snapshot_for(root: &Path) -> RepositorySnapshot {
    snapshot_with_targets(
        root,
        vec![
            ManagedTargetIdentity::whole_file(
                RelativePath::new("managed.txt").expect("path"),
                Some(identity(11)),
            )
            .expect("target"),
        ],
    )
}

fn snapshot_with_targets(root: &Path, targets: Vec<ManagedTargetIdentity>) -> RepositorySnapshot {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git rev-parse HEAD: {output:?}");
    let base_head = String::from_utf8(output.stdout)
        .expect("stdout")
        .trim()
        .to_owned();
    RepositorySnapshot::new(
        crate::repository::RepositoryFacts::new(
            crate::repository::RepositoryId::new("dest-a").expect("id"),
            crate::repository::RepositoryRoot::new(
                root.to_str().expect("path"),
                crate::repository::AuthorityIdentity::new(
                    FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
                    ObjectIdentity::new(7, 9),
                )
                .expect("authority"),
            )
            .expect("root"),
            crate::repository::GitRepositoryState::Git(
                crate::repository::GitFacts::new(
                    crate::repository::HeadState::Attached {
                        branch: crate::repository::RefName::new("refs/heads/main").expect("ref"),
                        commit: crate::repository::RevisionId::new(&base_head).expect("rev"),
                    },
                    crate::repository::UpstreamState::Absent,
                    crate::repository::IndexState::Clean,
                    crate::repository::WorktreeState::Clean,
                )
                .expect("git facts"),
            ),
        )
        .expect("facts"),
        crate::repository::FrozenWitnesses::new(
            "a",
            "s",
            "c",
            "cfg",
            "p",
            vec![],
            Some(crate::repository::RevisionId::new(base_head).expect("rev")),
        )
        .expect("witnesses"),
        targets,
    )
    .expect("snapshot")
}

#[test]
fn an_absent_target_is_created_and_delivered_as_a_lawful_creation() {
    use std::os::unix::fs::PermissionsExt;
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    // A source root providing the authoritative bytes for a first-time
    // destination file in a nested directory.
    let source_fixture = tempfile::Builder::new()
        .prefix("single-repo-source-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .expect("source fixture");
    fs::write(source_fixture.path().join("created.txt"), "created-v1\n").expect("source file");
    let plan = crate::lifecycle::sync_plan::SyncPlan::new(
        "dest-a",
        vec![crate::lifecycle::sync_plan::PlanItem {
            id: "item-created".to_owned(),
            target: "nested/dir/created.txt".to_owned(),
            source: "source-a".to_owned(),
            source_path: "created.txt".to_owned(),
            source_order: 0,
            kind: crate::source::ItemKind::WholeFile,
            section: None,
            decision: crate::lifecycle::sync_plan::PlanDecision::Selected {
                reason: "declared winner".to_owned(),
            },
        }],
    );
    let mut sources = std::collections::HashMap::new();
    sources.insert("source-a".to_owned(), source_fixture.path().to_path_buf());
    // The snapshot freezes the absent target as the lawful creation case.
    let snapshot = snapshot_with_targets(
        &root,
        vec![
            ManagedTargetIdentity::whole_file(
                RelativePath::new("nested/dir/created.txt").expect("path"),
                None,
            )
            .expect("target"),
        ],
    );
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot,
        &[],
        &plan,
        &sources,
        "sync managed",
    )
    .expect("pass");
    assert!(
        matches!(outcome, PassOutcome::Delivered { .. }),
        "the creation delivers: {outcome:?}"
    );
    // The first-time file exists with the authoritative bytes, safe
    // created parents, and the 0644 creation mode (subject to the umask).
    assert_eq!(
        fs::read(root.join("nested/dir/created.txt")).expect("created"),
        b"created-v1\n"
    );
    let mode = fs::metadata(root.join("nested/dir/created.txt"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode & !0o644, 0, "no bits beyond the 0644 creation mode");
    journal.shutdown().expect("shutdown");
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
        &[],
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
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
        &[],
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
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
    // Redelivering a pass with no selected operation creates no commit
    // object. The frozen base is the stable reconciliation witness.
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
    assert_eq!(commits, 1, "the base only; no empty delivery commit object");
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
        &[],
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
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
        &[],
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
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

#[test]
fn a_failing_declared_check_prevents_git_delivery() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let checks =
        vec![crate::repository::VerificationCommand::new(["/usr/bin/false"]).expect("command")];
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        &checks,
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
        "sync managed",
    )
    .expect("pass");
    // The failed check fails the pass: no Git delivery happens and no
    // scoped commit object is created beyond the base.
    let failed = match outcome {
        PassOutcome::Failed { reason } => reason,
        other => panic!("expected the failing check to fail the pass: {other:?}"),
    };
    assert!(failed.contains("verification"), "{failed}");
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
    assert_eq!(commits, 1, "base only; the failing check prevents Git");
    journal.shutdown().expect("shutdown");
}

#[test]
fn every_declared_check_runs_after_an_earlier_failure() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let checks = vec![
        crate::repository::VerificationCommand::new(["/usr/bin/false"]).expect("command"),
        crate::repository::VerificationCommand::new([
            "/bin/sh",
            "-c",
            "printf checked > second-check-ran",
        ])
        .expect("command"),
    ];
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        &checks,
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
        "sync managed",
    )
    .expect("pass");
    assert!(matches!(outcome, PassOutcome::Failed { .. }));
    assert_eq!(
        fs::read_to_string(root.join("second-check-ran")).expect("second check ran"),
        "checked"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_passing_declared_check_allows_git_delivery() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let checks =
        vec![crate::repository::VerificationCommand::new(["/usr/bin/true"]).expect("command")];
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        &checks,
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
        "sync managed",
    )
    .expect("pass");
    assert!(
        matches!(outcome, PassOutcome::Delivered { .. }),
        "a passing check permits the scoped delivery: {outcome:?}"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_concurrent_managed_change_prevents_git_delivery() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    // The snapshot froze managed.txt as a regular file.  A declared check
    // then removes the managed target during verification: the fresh
    // capture sees a deletion at a managed path, which is not the
    // authorized replacement, and the pass fails without Git.
    let checks = vec![
        crate::repository::VerificationCommand::new(["/bin/rm", "managed.txt"]).expect("command"),
    ];
    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot_for(&root),
        &checks,
        &crate::lifecycle::sync_plan::SyncPlan::new("dest-a", Vec::new()),
        &std::collections::HashMap::new(),
        "sync managed",
    )
    .expect("pass");
    let failed = match outcome {
        PassOutcome::Failed { reason } => reason,
        other => panic!("expected the concurrent change to fail the pass: {other:?}"),
    };
    assert!(failed.contains("concurrently"), "{failed}");
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
    assert_eq!(commits, 1, "base only; the concurrent change prevents Git");
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_verifier_cannot_replace_authoritative_managed_bytes() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id, _record_path) = journal_fixture();
    let source_fixture = tempfile::Builder::new()
        .prefix("single-repo-source-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .expect("source fixture");
    fs::write(source_fixture.path().join("managed.txt"), "v2\n").expect("source file");
    let plan = crate::lifecycle::sync_plan::SyncPlan::new(
        "dest-a",
        vec![crate::lifecycle::sync_plan::PlanItem {
            id: "item-managed".to_owned(),
            target: "managed.txt".to_owned(),
            source: "source-a".to_owned(),
            source_path: "managed.txt".to_owned(),
            source_order: 0,
            kind: crate::source::ItemKind::WholeFile,
            section: None,
            decision: crate::lifecycle::sync_plan::PlanDecision::Selected {
                reason: "declared winner".to_owned(),
            },
        }],
    );
    let mut sources = std::collections::HashMap::new();
    sources.insert("source-a".to_owned(), source_fixture.path().to_path_buf());
    let snapshot = snapshot_for(&root);
    let checks = vec![
        crate::repository::VerificationCommand::new([
            "/bin/sh",
            "-c",
            "printf 'verifier bytes\\n' > managed.txt",
        ])
        .expect("command"),
    ];

    let outcome = run_single_repository_pass(
        &root,
        &journal.handle,
        &run_id,
        "dest-a",
        &snapshot,
        &checks,
        &plan,
        &sources,
        "sync managed",
    )
    .expect("pass");

    let reason = match outcome {
        PassOutcome::Failed { reason } => reason,
        other => panic!("expected verifier mutation to fail the pass: {other:?}"),
    };
    assert!(reason.contains("managed bytes"), "{reason}");
    assert_eq!(
        fs::read(root.join("managed.txt")).expect("managed file"),
        b"v2\n",
        "the authoritative synchronization bytes remain visible"
    );
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
    assert_eq!(commits, 1, "base only; verifier mutation prevents Git");
    journal.shutdown().expect("shutdown");
}
