//! Focused proof for sanitized repository and Git state capture.

#![allow(dead_code, unused_imports)]

use super::capture::{CaptureError, capture_state};
use super::state::{
    GitRepositoryState, HeadState, IndexState, TargetChange, UpstreamState, WorktreeState,
};
use std::{fs, path::Path};

fn fixture_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("capture-home-")
        .tempdir_in(&base)
        .expect("create capture fixture");
    let root = fixture.path().join("repo");
    fs::create_dir_all(&root).expect("create repo");
    git(&root, &["init", "--quiet", "-b", "master"]);
    git(&root, &["config", "user.name", "Capture"]);
    git(&root, &["config", "user.email", "capture@example.test"]);
    (fixture, root)
}

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "--message", message]);
}

#[test]
fn non_git_directory_is_a_lawful_state() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let fixture = tempfile::Builder::new()
        .prefix("capture-nongit-")
        .tempdir_in(&base)
        .expect("fixture");
    assert_eq!(
        capture_state(fixture.path()).expect("capture"),
        GitRepositoryState::NonGit
    );
}

#[test]
fn clean_repository_captures_attached_head_and_clean_states() {
    let (_fixture, root) = fixture_repo();
    write(&root, "tracked.txt", "content\n");
    commit_all(&root, "base");
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    match facts.head() {
        HeadState::Attached { branch, commit } => {
            assert_eq!(branch.as_str(), "refs/heads/master");
            assert_eq!(commit.as_str().len(), 40);
        }
        other => panic!("expected attached head, got {other:?}"),
    }
    assert_eq!(facts.index(), &IndexState::Clean);
    assert_eq!(facts.worktree(), &WorktreeState::Clean);
}

#[test]
fn dirty_worktree_captures_modified_and_untracked() {
    let (_fixture, root) = fixture_repo();
    write(&root, "tracked.txt", "content\n");
    commit_all(&root, "base");
    write(&root, "tracked.txt", "changed\n");
    write(&root, "new.txt", "untracked\n");
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    let WorktreeState::Entries(entries) = facts.worktree() else {
        panic!("expected worktree entries");
    };
    let changes: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{}:{}",
                lossy(entry.path().as_bytes()),
                change_name(entry.change())
            )
        })
        .collect();
    assert!(
        changes.contains(&"tracked.txt:modified".to_owned()),
        "changes: {changes:?}"
    );
    assert!(
        changes.contains(&"new.txt:untracked".to_owned()),
        "changes: {changes:?}"
    );
}

#[test]
fn staged_add_and_delete_are_captured_in_the_index() {
    let (_fixture, root) = fixture_repo();
    write(&root, "tracked.txt", "content\n");
    commit_all(&root, "base");
    write(&root, "staged.txt", "new file\n");
    git(&root, &["add", "staged.txt"]);
    git(&root, &["rm", "--quiet", "tracked.txt"]);
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    let IndexState::Entries(entries) = facts.index() else {
        panic!("expected index entries");
    };
    let changes: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{}:{}",
                lossy(entry.path().as_bytes()),
                change_name(entry.change())
            )
        })
        .collect();
    assert!(
        changes.contains(&"staged.txt:added".to_owned()),
        "{changes:?}"
    );
    assert!(
        changes.contains(&"tracked.txt:deleted".to_owned()),
        "{changes:?}"
    );
}

#[test]
fn rename_is_captured_with_both_paths() {
    let (_fixture, root) = fixture_repo();
    write(&root, "old.txt", "content\n");
    commit_all(&root, "base");
    // A staged rename (git mv) is an index entry carrying both paths.
    git(&root, &["mv", "old.txt", "new.txt"]);
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    let IndexState::Entries(entries) = facts.index() else {
        panic!("expected index entries");
    };
    let renamed: Vec<_> = entries
        .iter()
        .filter(|entry| entry.change() == TargetChange::Renamed)
        .collect();
    assert_eq!(renamed.len(), 1, "renames: {entries:?}");
    assert_eq!(lossy(renamed[0].path().as_bytes()), "new.txt");
    assert_eq!(
        lossy(renamed[0].rename_from().expect("rename source").as_bytes()),
        "old.txt"
    );
}

#[test]
fn type_change_is_captured() {
    let (_fixture, root) = fixture_repo();
    write(&root, "link.txt", "content\n");
    commit_all(&root, "base");
    fs::remove_file(root.join("link.txt")).expect("remove file");
    std::os::unix::fs::symlink("target", root.join("link.txt")).expect("symlink");
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    let WorktreeState::Entries(entries) = facts.worktree() else {
        panic!("expected worktree entries");
    };
    assert!(
        entries
            .iter()
            .any(|entry| entry.change() == TargetChange::TypeChanged),
        "type change must be captured: {entries:?}"
    );
}

#[test]
fn unborn_and_detached_heads_are_captured() {
    let (_fixture, root) = fixture_repo();
    // Unborn: no commits yet.
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    assert_eq!(facts.head(), &HeadState::Unborn);

    // Detached: check out a commit directly.
    write(&root, "a.txt", "a\n");
    commit_all(&root, "base");
    git(&root, &["checkout", "--quiet", "HEAD~0"]);
    git(&root, &["checkout", "--quiet", "--detach"]);
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    assert!(matches!(facts.head(), HeadState::Detached { .. }));
}

#[test]
fn configured_upstream_is_captured() {
    let (fixture, root) = fixture_repo();
    write(&root, "a.txt", "a\n");
    commit_all(&root, "base");
    // A real bare remote so the upstream commit resolves.
    let bare = fixture.path().join("origin.git");
    let output = std::process::Command::new("git")
        .args(["init", "--quiet", "--bare", "-b", "master"])
        .arg(&bare)
        .output()
        .expect("spawn bare init");
    assert!(output.status.success(), "bare init failed");
    git(
        &root,
        &["remote", "add", "origin", bare.to_str().expect("bare path")],
    );
    let output = std::process::Command::new("git")
        .args(["push", "--quiet", "-u", "origin", "master"])
        .current_dir(&root)
        .output()
        .expect("git push");
    assert!(output.status.success(), "push failed: {:?}", output);
    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    match facts.upstream() {
        UpstreamState::Configured {
            remote,
            reference,
            commit,
        } => {
            assert_eq!(remote, "origin");
            assert_eq!(reference.as_str(), "refs/remotes/origin/master");
            assert_eq!(commit.as_str().len(), 40);
        }
        other => panic!("expected configured upstream, got {other:?}"),
    }
}

#[test]
fn hostile_config_cannot_execute_or_falsify_scope() {
    let (_fixture, root) = fixture_repo();
    // The include file is tracked so the worktree stays clean; the hostile
    // configs themselves are then pointed at it.
    write(&root, "a.txt", "a\n");
    write(
        &root,
        "evil.inc",
        "[core]\n\thooksPath = /tmp/omnirepo-capture-hooks\n",
    );
    write(
        &root,
        "evil-fsmonitor.sh",
        "#!/bin/sh\ntouch /tmp/omnirepo-capture-executed\n",
    );
    write(
        &root,
        "evil-pager.sh",
        "#!/bin/sh\ntouch /tmp/omnirepo-capture-pager\n",
    );
    commit_all(&root, "base");
    // A repository-controlled config that tries to run a process through
    // core.fsmonitor and pager, plus an include of the attacker file.
    git(&root, &["config", "core.fsmonitor", "evil-fsmonitor.sh"]);
    git(&root, &["config", "pager.status", "evil-pager.sh"]);
    git(&root, &["config", "include.path", "evil.inc"]);

    let GitRepositoryState::Git(facts) = capture_state(&root).expect("capture") else {
        panic!("expected git state");
    };
    assert_eq!(facts.worktree(), &WorktreeState::Clean);
    assert!(
        !Path::new("/tmp/omnirepo-capture-executed").exists(),
        "core.fsmonitor must not execute"
    );
    assert!(
        !Path::new("/tmp/omnirepo-capture-pager").exists(),
        "pager must not execute"
    );
    assert!(
        !Path::new("/tmp/omnirepo-capture-hooks").exists(),
        "config include must not redirect hooks"
    );
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn change_name(change: TargetChange) -> &'static str {
    match change {
        TargetChange::Added => "added",
        TargetChange::Deleted => "deleted",
        TargetChange::Modified => "modified",
        TargetChange::Renamed => "renamed",
        TargetChange::TypeChanged => "type-changed",
        TargetChange::ModeChanged => "mode-changed",
        TargetChange::LinkChanged => "link-changed",
        TargetChange::Untracked => "untracked",
    }
}
