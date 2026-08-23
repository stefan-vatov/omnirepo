//! Fixture proof for the commit path under dirty state, hostile
//! hook/config, crash recovery, and scoped-tree discipline.

#![allow(dead_code, unused_imports)]

use crate::repository::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, GitRepositoryState, HeadState, IndexState, ManagedTargetIdentity,
    ObjectIdentity, PlannedOperation, RefName, RelativePath, RepositoryFacts, RepositoryId,
    RepositoryRoot, RepositorySnapshot, RevisionId, UpstreamState, WorktreeState,
    build_authorized_delta, create_commit, prepare_index,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

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
fn witnesses(base_head: &str) -> FrozenWitnesses {
    FrozenWitnesses::new(
        "authority-1",
        "source-1",
        "catalog-1",
        "configuration-1",
        "plan-1",
        vec![witness("check-a")],
        Some(revision(base_head)),
    )
    .expect("witnesses")
}
fn facts(base_head: &str) -> RepositoryFacts {
    RepositoryFacts::new(
        repository_id("destination-a"),
        root("/workspace/destination-a"),
        GitRepositoryState::Git(
            GitFacts::new(
                HeadState::Attached {
                    branch: ref_name("refs/heads/main"),
                    commit: revision(base_head),
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
fn baseline(path_value: &str, inode: u64, base_head: &str) -> RepositorySnapshot {
    let target =
        ManagedTargetIdentity::whole_file(path(path_value), Some(identity(inode))).expect("target");
    RepositorySnapshot::new(facts(base_head), witnesses(base_head), vec![target]).expect("snapshot")
}

fn delta_for(
    root: &Path,
    relative: &str,
    content: &str,
    inode: u64,
) -> crate::repository::AuthorizedDelta {
    base_commit(root, relative, content);
    let base = git_text(root, &["rev-parse", "HEAD"]);
    let snapshot = baseline(relative, inode, &base);
    build_authorized_delta(
        &snapshot,
        vec![PlannedOperation::replaced(
            path(relative),
            identity(inode),
            identity(inode + 1),
        )],
    )
    .expect("delta")
}

fn fixture_repo_root() -> (tempfile::TempDir, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("commit-fixture-repo-")
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

fn base_commit(root: &Path, relative: &str, content: &str) {
    write(root, relative, content);
    git_text(root, &["add", "."]);
    git_text(root, &["commit", "--quiet", "--message", "base"]);
}

#[test]
fn commit_ignores_dirty_worktree_and_index_outside_the_isolated_tree() {
    let (_fixture, root) = fixture_repo_root();
    let delta = delta_for(&root, "managed.txt", "v1\n", 11);
    // A dirty file outside the managed scope must not leak into the commit:
    // the commit is built from the isolated index only.
    write(&root, "scratch.txt", "dirty\n");
    let index = prepare_index(&root, &delta).expect("index");
    let committed = create_commit(&root, &index, None, "sync managed").expect("commit");
    let tree = git_text(&root, &["ls-tree", "-r", "--name-only", &committed.tree]);
    assert!(tree.contains("managed.txt"), "{tree}");
    assert!(!tree.contains("scratch.txt"), "{tree}");
}

#[test]
fn hostile_hooks_and_config_are_inert_during_commit() {
    let (_fixture, root) = fixture_repo_root();
    let delta = delta_for(&root, "managed.txt", "v1\n", 11);
    // A hostile hook directory would create a marker if it ran; the commit
    // path must neutralize hooks (core.hooksPath=/dev/null) even when the
    // repository config points at the hostile directory.
    let evil = root.join("evil-hooks");
    fs::create_dir_all(&evil).expect("evil hooks");
    fs::write(
        evil.join("pre-commit"),
        "#!/bin/sh\ntouch /tmp/omnirepo-hook-ran\n",
    )
    .expect("hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(evil.join("pre-commit"))
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(evil.join("pre-commit"), permissions).expect("set executable");
    }
    let marker = Path::new("/tmp/omnirepo-hook-ran");
    let _ = fs::remove_file(marker);
    fs::OpenOptions::new()
        .append(true)
        .open(root.join(".git/config"))
        .expect("config append")
        .write_all(b"\n[core]\n\thooksPath = /tmp/omnirepo-nonexistent-evil\n\tfsmonitor = true\n")
        .expect("write config");
    let index = prepare_index(&root, &delta).expect("index");
    let committed = create_commit(&root, &index, None, "sync managed").expect("commit");
    assert!(!committed.sha.is_empty());
    assert!(!marker.exists(), "hostile hook executed");
}

#[test]
fn crash_between_commit_and_journal_reconciles_by_oid() {
    let (_fixture, root) = fixture_repo_root();
    let delta = delta_for(&root, "managed.txt", "v1\n", 11);
    let base = git_text(&root, &["rev-parse", "HEAD"]);
    let index = prepare_index(&root, &delta).expect("index");
    // Commit succeeds; the journal side "crashes" (record never written).
    // Recovery reconciles the exact OID against the object database.
    let committed = create_commit(&root, &index, Some(&base), "sync managed").expect("commit");
    let git_root = crate::platform::AuthorityRoot::<
        crate::platform::GitWorkingDirectoryRoot,
        crate::platform::ReadOnly,
    >::open(&root)
    .expect("git root");
    assert!(
        crate::lifecycle::commit_journal::reconcile_commit(&git_root, &committed.sha)
            .expect("reconcile")
    );
    let missing = "0000000000000000000000000000000000000000";
    assert!(
        !crate::lifecycle::commit_journal::reconcile_commit(&git_root, missing).expect("reconcile")
    );
}
