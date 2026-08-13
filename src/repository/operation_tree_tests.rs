//! Focused proof for the exact operation-scoped candidate tree.

#![allow(dead_code, unused_imports)]

use super::manifest::PlannedOperation;
use super::operation_tree::{OperationTree, TreeEntry, TreeError, build_operation_tree};
use super::state::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, GitRepositoryState, HeadState, IndexState, ManagedTargetIdentity,
    ObjectIdentity, RefName, RepositoryFacts, RepositoryId, RepositoryRoot, RepositorySnapshot,
    RevisionId, TargetChange, UpstreamState, WorktreeState,
};
use std::{fs, path::Path, process::Command};

fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("repository id")
}
fn root(value: &str) -> RepositoryRoot {
    RepositoryRoot::new(value, authority(0)).expect("repository root")
}
fn path(value: &str) -> super::state::RelativePath {
    super::state::RelativePath::from_bytes(value).expect("relative path")
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

fn fixture_repo_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("tree-home-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("repo");
    fs::create_dir_all(&root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Tree"]);
    git(&["config", "user.email", "tree@example.test"]);
    (fixture, root)
}

fn write(root: &Path, relative: &str, content: &str) {
    fs::write(root.join(relative), content).expect("write");
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
fn tree_contains_every_and_only_authorized_entries() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    write(&root, "other.txt", "other\n");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    write(&root, "managed.txt", "v2\n");

    let snapshot = baseline("managed.txt", 11);
    let delta = super::manifest::build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let tree = build_operation_tree(&root, &delta).expect("tree");
    assert_eq!(tree.entries.len(), 1, "every and only authorized: {tree:?}");
    match &tree.entries[0] {
        TreeEntry::Blob { path, blob } => {
            assert_eq!(path, "managed.txt");
            assert_eq!(
                blob,
                &git_text(&root, &["hash-object", "--", "managed.txt"])
            );
        }
        other => panic!("expected blob entry, got {other:?}"),
    }
}

#[test]
fn deletions_and_renames_are_exact_and_drift_fails() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);

    let snapshot = baseline("managed.txt", 11);
    let deleted = super::manifest::build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::deleted(path("managed.txt"), identity(11))],
    )
    .expect("delta");
    let tree = build_operation_tree(&root, &deleted).expect("tree");
    assert_eq!(
        tree.entries,
        vec![TreeEntry::Removal {
            path: "managed.txt".to_owned()
        }]
    );

    // An authorized non-deleted change with a missing worktree file is drift.
    fs::remove_file(root.join("managed.txt")).expect("remove");
    let replaced = super::manifest::build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let error = build_operation_tree(&root, &replaced).expect_err("drift must fail");
    assert!(matches!(error, TreeError::Drift { .. }), "{error:?}");
}

#[test]
fn unrelated_staged_content_never_enters_the_tree() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    write(&root, "managed.txt", "v2\n");
    write(&root, "unrelated.txt", "staged elsewhere\n");
    git(&["add", "unrelated.txt"]);

    let snapshot = baseline("managed.txt", 11);
    let delta = super::manifest::build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let tree = build_operation_tree(&root, &delta).expect("tree");
    assert_eq!(
        tree.entries.len(),
        1,
        "unrelated staged content excluded: {tree:?}"
    );
    assert!(
        !tree
            .entries
            .iter()
            .any(|entry| matches!(entry, TreeEntry::Blob { path, .. } if path == "unrelated.txt")),
        "{tree:?}"
    );
}
