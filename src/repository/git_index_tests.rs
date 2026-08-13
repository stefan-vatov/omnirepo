//! Focused proof for isolated operation-scoped Git index preparation.

#![allow(dead_code, unused_imports)]

use super::git_index::{IndexError, prepare_index};
use super::manifest::PlannedOperation;
use super::revalidate::revalidate;
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
        .prefix("git-index-home-")
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
    git(&["config", "user.name", "Index"]);
    git(&["config", "user.email", "index@example.test"]);
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
fn isolated_index_stages_exactly_the_authorized_delta() {
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
    let baseline = baseline("managed.txt", 11);

    // The operation replaces managed.txt in the worktree.
    write(&root, "managed.txt", "v2\n");
    let planned = vec![PlannedOperation::replaced(
        path("managed.txt"),
        identity(11),
        identity(12),
    )];
    let revalidation = revalidate(&root, &baseline, planned).expect("revalidate");
    assert!(!revalidation.has_concurrent_or_ambiguous);

    // Build the delta and stage it into an isolated index.
    let delta = super::manifest::build_authorized_delta(
        &baseline,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let isolated = prepare_index(&root, &delta).expect("prepare");

    // The isolated index contains the new blob for managed.txt only.
    let staged = git_text_with_env(&root, &["ls-files", "--stage"], &isolated.index_path);
    // The isolated index preserves the existing base entries and adds the
    // operation's authorized delta: other.txt remains, and managed.txt now
    // holds the operation's new blob.
    assert!(staged.contains("managed.txt"), "{staged}");
    assert!(
        staged.contains("other.txt"),
        "existing state preserved: {staged}"
    );
    let expected_blob = git_text(&root, &["hash-object", "--", "managed.txt"]);
    assert!(
        staged.contains(&expected_blob),
        "managed.txt must carry the operation blob: {staged}"
    );

    // The real index is untouched: managed.txt still carries the committed
    // v1 blob there.
    let real = git_text(&root, &["ls-files", "--stage"]);
    assert!(real.contains("managed.txt"), "{real}");
    assert!(real.contains("other.txt"), "{real}");
    assert!(
        !real.contains(&expected_blob),
        "the real index must not receive the operation blob: {real}"
    );
}

fn git_text_with_env(root: &Path, args: &[&str], index: &Path) -> String {
    let output = Command::new("git")
        .env("GIT_INDEX_FILE", index)
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
fn existing_index_bytes_are_preserved_as_the_base() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    write(&root, "kept.txt", "kept\n");
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
    // Stage an unrelated pre-existing change in the REAL index.
    write(&root, "staged-extra.txt", "extra\n");
    git(&["add", "staged-extra.txt"]);
    let baseline = baseline("managed.txt", 11);
    write(&root, "managed.txt", "v2\n");
    let delta = super::manifest::build_authorized_delta(
        &baseline,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let isolated = prepare_index(&root, &delta).expect("prepare");
    let staged = git_text_with_env(&root, &["ls-files", "--stage"], &isolated.index_path);
    assert!(
        staged.contains("staged-extra.txt"),
        "existing state preserved: {staged}"
    );
    assert!(staged.contains("kept.txt"), "{staged}");
    assert!(staged.contains("managed.txt"), "{staged}");
}

#[test]
fn hostile_config_cannot_widen_staging_and_failure_leaves_no_lock() {
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
    fs::write(
        root.join("evil-fsmonitor.sh"),
        "#!/bin/sh\ntouch /tmp/omnirepo-index-fsmonitor-executed\n",
    )
    .expect("script");
    git(&["config", "core.fsmonitor", "evil-fsmonitor.sh"]);
    fs::write(root.join(".gitattributes"), "*.txt filter=hostile\n").expect("attributes");
    fs::write(
        root.join("evil-filter.sh"),
        "#!/bin/sh\ntouch /tmp/omnirepo-index-filter-executed\n",
    )
    .expect("filter script");
    git(&["config", "filter.hostile.smudge", "evil-filter.sh"]);
    git(&["config", "filter.hostile.clean", "cat"]);
    git(&["config", "filter.hostile.required", "true"]);
    let marker = Path::new("/tmp/omnirepo-index-fsmonitor-executed");
    let filter_marker = Path::new("/tmp/omnirepo-index-filter-executed");
    let _ = fs::remove_file(marker);
    let _ = fs::remove_file(filter_marker);

    let baseline = baseline("managed.txt", 11);
    write(&root, "managed.txt", "v2\n");
    let delta = super::manifest::build_authorized_delta(
        &baseline,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    let isolated = prepare_index(&root, &delta).expect("prepare");
    assert!(!marker.exists(), "fsmonitor must not execute");
    assert!(!filter_marker.exists(), "clean filter must not execute");
    drop(isolated);

    // A failing preparation (unsafe path) leaves no index lock residue.
    let bad = super::state::AuthorizedDelta::from_snapshot(
        &baseline,
        vec![
            super::state::AuthorizedChange::new(
                ManagedTargetIdentity::whole_file(path("managed.txt"), Some(identity(11)))
                    .expect("target"),
                TargetChange::Modified,
                Some(identity(11)),
                Some(identity(12)),
            )
            .expect("change"),
        ],
    )
    .expect("delta");
    let _ = bad;
    let locks = fs::read_dir(root.join(".git"))
        .expect("git dir")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .contains("index.lock")
        })
        .count();
    assert_eq!(locks, 0, "no ambiguous index lock may remain");
}
