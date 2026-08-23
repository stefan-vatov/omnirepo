//! Focused proof for the owner-contracted operation commit.

#![allow(dead_code, unused_imports)]

use crate::repository::manifest::PlannedOperation;
use crate::repository::state::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, GitRepositoryState, HeadState, IndexState, ManagedTargetIdentity,
    ObjectIdentity, RefName, RepositoryFacts, RepositoryId, RepositoryRoot, RepositorySnapshot,
    RevisionId, UpstreamState, WorktreeState,
};
use std::{fs, path::Path, path::PathBuf, process::Command};

fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("repository id")
}
fn root(value: &str) -> RepositoryRoot {
    RepositoryRoot::new(value, authority(0)).expect("repository root")
}
fn path(value: &str) -> crate::repository::state::RelativePath {
    crate::repository::state::RelativePath::from_bytes(value).expect("relative path")
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
        FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
        ObjectIdentity::new(7, inode),
        EntryKind::RegularFile,
        0o100644,
    )
    .expect("file identity")
}
fn authority(inode: u64) -> AuthorityIdentity {
    AuthorityIdentity::new(
        FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
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

fn fixture_repo_root() -> (tempfile::TempDir, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("commit-home-")
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
fn operation_commit_records_exact_tree_without_widening_effects() {
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
    let delta = crate::repository::manifest::build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let isolated = crate::repository::git_index::prepare_index(&root, &delta).expect("index");
    let recorded = crate::repository::operation_commit::create_commit(
        &root,
        &isolated,
        Some(&base),
        "chore(omnirepo): sync managed content",
    )
    .expect("commit");
    assert_eq!(recorded.parent.as_deref(), Some(base.as_str()));
    // The commit object exists with the exact tree.
    let tree_sha = git_text(
        &root,
        &["rev-parse", &format!("{}^0^{{tree}}", recorded.sha)],
    );
    assert_eq!(tree_sha, recorded.tree);
    // No widening effects: the branch still points at the base commit and
    // the worktree/index are untouched.
    assert_eq!(git_text(&root, &["rev-parse", "HEAD"]), base);
    let status = git_text(&root, &["status", "--porcelain"]);
    assert!(
        status.contains("managed.txt"),
        "the operation worktree change shows: {status}"
    );
    assert!(!status.contains("other"), "no widening effects: {status}");
    let real_index = git_text(&root, &["ls-files", "--stage"]);
    assert!(
        !real_index.contains(&git_text(&root, &["hash-object", "--", "managed.txt"])),
        "the real index must not receive the operation blob"
    );
}
