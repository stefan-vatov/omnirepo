//! Focused proof for scoped Git delivery after a verified pass.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::git_delivery::{DeliveryOutcome, coordinate_git_delivery};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::lifecycle::verify_and_gate::VerificationVerdict;
use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use crate::repository::{
    EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity, ManagedTargetIdentity,
    ObjectIdentity, ObjectIdentity as _O, RelativePath, RepositorySnapshot, RevisionId,
};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn journal_fixture() -> (tempfile::TempDir, Journal, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("git-delivery-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [3_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id)
}

fn git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("git-delivery-repo-")
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
    let upstream = fixture.path().join("upstream.git");
    git(&[
        "init",
        "--quiet",
        "--bare",
        upstream.to_str().expect("path"),
    ]);
    git(&["remote", "add", "origin", upstream.to_str().expect("path")]);
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
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

fn target_for(_root: &Path) -> ManagedTargetIdentity {
    ManagedTargetIdentity::whole_file(
        RelativePath::new("managed.txt").expect("path"),
        Some(identity(11)),
    )
    .expect("target")
}

fn head(root: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git rev-parse HEAD: {output:?}");
    String::from_utf8(output.stdout)
        .expect("stdout")
        .trim()
        .to_owned()
}

#[test]
fn a_verified_pass_commits_the_scoped_delta_and_reconciles() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let base = head(&root);
    fs::write(root.join("managed.txt"), "v2\n").expect("file");
    let snapshot = RepositorySnapshot::new(
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
                        commit: crate::repository::RevisionId::new(&base).expect("rev"),
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
            Some(crate::repository::RevisionId::new(&base).expect("rev")),
        )
        .expect("witnesses"),
        vec![target_for(&root)],
    )
    .expect("snapshot");
    let delta = crate::repository::build_authorized_delta(
        &snapshot,
        vec![crate::repository::PlannedOperation::replaced(
            RelativePath::new("managed.txt").expect("path"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let index = crate::repository::prepare_index(&root, &delta).expect("index");
    let git_root = AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&root).expect("root");
    let outcome = coordinate_git_delivery(
        &git_root,
        &index,
        Some(&base),
        "sync managed",
        &journal.handle,
        &run_id,
        "dest-a",
        VerificationVerdict::Ready,
    )
    .expect("delivery");
    assert!(matches!(outcome, DeliveryOutcome::Delivered { .. }));
    // The exact commit OID exists in the object database.
    match outcome {
        DeliveryOutcome::Delivered { oid } => {
            assert!(
                crate::lifecycle::commit_journal::reconcile_commit(&git_root, &oid)
                    .expect("reconcile"),
                "the delivered OID must exist"
            );
        }
        other => panic!("expected delivered, got {other:?}"),
    }
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_non_ready_verdict_prevents_git_contact() {
    let (_fixture, root) = git_repo();
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let base = head(&root);
    let git_root = AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&root).expect("root");
    // The index is never prepared: delivery is refused before any contact.
    let outcome = coordinate_git_delivery(
        &git_root,
        &crate::repository::prepare_index(
            &root,
            &crate::repository::build_authorized_delta(
                &RepositorySnapshot::new(
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
                                    branch: crate::repository::RefName::new("refs/heads/main")
                                        .expect("ref"),
                                    commit: crate::repository::RevisionId::new(&base).expect("rev"),
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
                        Some(crate::repository::RevisionId::new(&base).expect("rev")),
                    )
                    .expect("witnesses"),
                    vec![target_for(&root)],
                )
                .expect("snapshot"),
                vec![crate::repository::PlannedOperation::replaced(
                    RelativePath::new("managed.txt").expect("path"),
                    identity(11),
                    identity(12),
                )],
            )
            .expect("delta"),
        )
        .expect("index"),
        None,
        "sync managed",
        &journal.handle,
        &run_id,
        "dest-a",
        VerificationVerdict::FailedCheck,
    )
    .expect("refusal is not an error");
    assert!(matches!(outcome, DeliveryOutcome::Rejected { .. }));
    journal.shutdown().expect("shutdown");
}
