//! Idempotence and journal identity fixtures.

use crate::managed_content::classify_whole_file;
use std::{fs, path::Path, process::Command};

fn git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("sync-idempotence-")
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

#[test]
fn second_run_performs_no_content_or_git_mutation() {
    let (_fixture, root) = git_repo();
    let target = root.join("managed.txt");
    // First run: the file is missing → create.
    let first = classify_whole_file(false, None, b"v1\n").expect("classify");
    assert_eq!(first, crate::managed_content::WholeFileOutcome::Create);
    fs::write(&target, "v1\n").expect("write");
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
    git(&["commit", "--quiet", "--message", "first sync"]);
    let commits_before = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&root)
        .output()
        .expect("git");
    let commits_before = String::from_utf8(commits_before.stdout).expect("stdout");
    // Second run: equal bytes → unchanged → no write, no commit.
    let second = classify_whole_file(true, Some(b"v1\n"), b"v1\n").expect("classify");
    assert_eq!(second, crate::managed_content::WholeFileOutcome::Unchanged);
    let content_after = fs::read_to_string(&target).expect("read");
    assert_eq!(content_after, "v1\n");
    let commits_after = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&root)
        .output()
        .expect("git");
    assert_eq!(
        commits_before,
        String::from_utf8(commits_after.stdout).expect("stdout"),
        "no Git mutation on the second run"
    );
}
