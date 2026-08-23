//! Focused proof for authorized-delta manifests and boundary revalidation.

#![allow(dead_code, unused_imports)]

use super::manifest::{ManifestError, PlannedOperation, build_authorized_delta};
use super::revalidate::{Classification, RevalidateError, revalidate};
use super::state::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, GitRepositoryState, HeadState, IndexState, ManagedTargetIdentity,
    ObjectIdentity, RefName, RepositoryFacts, RepositoryId, RepositoryRoot, RepositorySnapshot,
    RevisionId, TargetChange, UpstreamState, WorktreeState,
};
use std::path::Path;

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

fn fixture_repo_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("revalidate-home-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("repo");
    std::fs::create_dir_all(&root).expect("repo");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Revalidate"]);
    git(&["config", "user.email", "revalidate@example.test"]);
    (fixture, root)
}

fn write(root: &Path, relative: &str, content: &str) {
    std::fs::write(root.join(relative), content).expect("write");
}

#[test]
fn replaced_target_builds_an_authorized_modified_change() {
    let baseline = baseline("managed.txt", 11);
    let delta = build_authorized_delta(
        &baseline,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(11),
            identity(12),
        )],
    )
    .expect("delta");
    assert_eq!(delta.repository_id().as_str(), "destination-a");
    assert_eq!(delta.base_head(), Some(&revision("base-1")));
    assert_eq!(delta.witnesses().checks().len(), 1);
    assert_eq!(delta.changes().len(), 1);
    assert_eq!(delta.changes()[0].change(), TargetChange::Modified);
    assert_eq!(
        delta.changes()[0].target().observed_file(),
        Some(&identity(11)),
        "the before identity binds to the frozen baseline"
    );
    // The snapshot identity and authority are bound.
    assert_eq!(delta.authority_identity(), &authority(0));
}

#[test]
fn added_and_deleted_targets_build_explicit_changes() {
    // The baseline freezes the future target as absent (observed None).
    let absent_target =
        ManagedTargetIdentity::whole_file(path("new.txt"), None).expect("absent target");
    let empty_baseline =
        RepositorySnapshot::new(facts(), witnesses(), vec![absent_target]).expect("baseline");
    let delta = build_authorized_delta(
        &empty_baseline,
        vec![PlannedOperation::added(path("new.txt"), identity(12))],
    )
    .expect("delta");
    assert_eq!(delta.changes().len(), 1);
    assert_eq!(delta.changes()[0].change(), TargetChange::Added);
    assert!(delta.changes()[0].target().observed_file().is_none());

    let baseline = baseline("gone.txt", 11);
    let delta = build_authorized_delta(
        &baseline,
        vec![PlannedOperation::deleted(path("gone.txt"), identity(11))],
    )
    .expect("delta");
    assert_eq!(delta.changes().len(), 1);
    assert_eq!(delta.changes()[0].change(), TargetChange::Deleted);
}

#[test]
fn empty_operations_build_an_explicit_empty_delta() {
    let baseline = baseline("managed.txt", 11);
    let delta = build_authorized_delta(&baseline, Vec::new()).expect("empty delta");
    assert!(delta.changes().is_empty(), "empty delta is explicit");
    assert_eq!(delta.frozen_targets().len(), 1);
}

#[test]
fn unfrozen_and_mismatched_operations_fail_closed() {
    let baseline = baseline("managed.txt", 11);
    // An unfrozen path is unauthorized.
    let error = build_authorized_delta(
        &baseline,
        vec![PlannedOperation::deleted(path("other.txt"), identity(11))],
    )
    .expect_err("unfrozen target must fail");
    assert!(
        matches!(error, ManifestError::UnauthorizedTarget { .. }),
        "{error:?}"
    );
    // A before identity that does not match the frozen baseline is a
    // baseline mismatch (the baseline changed after capture).
    let error = build_authorized_delta(
        &baseline,
        vec![PlannedOperation::replaced(
            path("managed.txt"),
            identity(99),
            identity(12),
        )],
    )
    .expect_err("baseline mismatch must fail");
    assert!(
        matches!(error, ManifestError::BaselineMismatch { .. }),
        "{error:?}"
    );
}

#[test]
fn revalidation_classifies_expected_pre_existing_concurrent_and_missing() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    write(&root, "notes.txt", "notes\n");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    let baseline = baseline("managed.txt", 11);

    // The planned operation replaces managed.txt; the current state has only
    // that change (the operation landed) plus the unmanaged notes file.
    write(&root, "managed.txt", "v2\n");
    let planned = vec![PlannedOperation::replaced(
        path("managed.txt"),
        identity(11),
        identity(12),
    )];
    let result = revalidate(&root, &baseline, planned).expect("revalidate");
    assert!(!result.has_concurrent_or_ambiguous, "{result:?}");
    let managed = result
        .paths
        .iter()
        .find(|entry| entry.path == "managed.txt")
        .expect("managed entry");
    assert_eq!(managed.classification, Classification::ExpectedOperation);

    // A concurrent user edit to managed content outside the delta fails.
    write(&root, "notes.txt", "changed\n");
    let planned = Vec::new();
    let result = revalidate(&root, &baseline, planned).expect("revalidate");
    assert!(result.has_concurrent_or_ambiguous);
    let notes = result
        .paths
        .iter()
        .find(|entry| entry.path == "notes.txt")
        .expect("notes entry");
    assert_eq!(
        notes.classification,
        Classification::PreExisting,
        "unmanaged change"
    );
    let managed = result
        .paths
        .iter()
        .find(|entry| entry.path == "managed.txt")
        .expect("managed entry");
    assert_eq!(
        managed.classification,
        Classification::ConcurrentUserChange,
        "managed change outside the delta is concurrent"
    );

    // A planned change that did not land is a missing operation effect.
    git(&["checkout", "--quiet", "--", "managed.txt"]);
    let planned = vec![PlannedOperation::replaced(
        path("managed.txt"),
        identity(11),
        identity(12),
    )];
    let result = revalidate(&root, &baseline, planned).expect("revalidate");
    assert!(result.has_concurrent_or_ambiguous);
    assert!(
        result
            .paths
            .iter()
            .any(|entry| entry.classification == Classification::MissingOperationEffect),
        "{result:?}"
    );
}

#[test]
fn non_git_directory_revalidates_with_no_changes() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let fixture = tempfile::Builder::new()
        .prefix("revalidate-nongit-")
        .tempdir_in(&base)
        .expect("fixture");
    let baseline = baseline("managed.txt", 11);
    let result = revalidate(fixture.path(), &baseline, Vec::new()).expect("revalidate");
    assert!(!result.has_concurrent_or_ambiguous);
    assert!(result.paths.is_empty());
}

#[test]
fn hostile_worktree_combinations_classify_completely() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    write(&root, "other.txt", "o1\n");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    let baseline = baseline("managed.txt", 11);

    // A hostile combination: staged modify, unstaged modify, untracked file,
    // and a staged delete — only managed.txt is planned.
    write(&root, "managed.txt", "staged\n");
    git(&["add", "managed.txt"]);
    write(&root, "managed.txt", "unstaged\n");
    write(&root, "untracked.txt", "new\n");
    write(&root, "other.txt", "changed\n");
    git(&["add", "other.txt"]);
    git(&["rm", "--quiet", "--cached", "other.txt"]);
    let planned = vec![PlannedOperation::replaced(
        path("managed.txt"),
        identity(11),
        identity(12),
    )];
    let result = revalidate(&root, &baseline, planned).expect("revalidate");
    // managed.txt appears (index staged + worktree unstaged both map to
    // modified and classify as expected); the delete of other.txt (a managed
    // target outside the delta) is concurrent; untracked is pre-existing.
    let managed = result
        .paths
        .iter()
        .filter(|entry| entry.path == "managed.txt")
        .collect::<Vec<_>>();
    assert!(!managed.is_empty(), "{result:?}");
    assert!(
        result
            .paths
            .iter()
            .any(|entry| entry.path == "untracked.txt"),
        "{result:?}"
    );
}

#[test]
fn hostile_config_never_executes_during_revalidation() {
    let (_fixture, root) = fixture_repo_root();
    write(&root, "managed.txt", "v1\n");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    std::fs::write(
        root.join("evil-fsmonitor.sh"),
        "#!/bin/sh\ntouch /tmp/omnirepo-revalidate-fsmonitor-executed\n",
    )
    .expect("script");
    git(&["config", "core.fsmonitor", "evil-fsmonitor.sh"]);
    let marker = Path::new("/tmp/omnirepo-revalidate-fsmonitor-executed");
    let _ = std::fs::remove_file(marker);
    let baseline = baseline("managed.txt", 11);
    let result = revalidate(&root, &baseline, Vec::new()).expect("revalidate");
    assert!(
        !marker.exists(),
        "fsmonitor must never execute during revalidation"
    );
    assert!(
        !result.has_concurrent_or_ambiguous,
        "clean repo stays clean"
    );
}
