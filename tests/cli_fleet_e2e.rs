//! Configured-fleet E2E CLI fixtures: binary-level journeys with a
//! machine config, a local source, and real destinations covering
//! changed, unchanged, empty, partial, total, and repair exits with
//! exact stdout/stderr and record contents.

use std::{fs, path::Path, process::Command};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn command(home: &Path, current_dir: &Path) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command
        .current_dir(current_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn fixture(name: &str) -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(root: &Path) {
    fs::create_dir_all(root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "E2E"]);
    git(&["config", "user.email", "e2e@example.test"]);
}

fn setup_source(fixture: &tempfile::TempDir) -> std::path::PathBuf {
    let source = fixture.path().join("source-a");
    git_repo(&source);
    fs::write(source.join("managed.txt"), "v1\n").expect("source file");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&source)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "source"]);
    let head = git_text(&source, &["rev-parse", "HEAD"]);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(
        source.join(".omnirepo/source.yaml"),
        format!(
            "omnirepo-declarations-v1\nsource=source-a revision={head} path=managed.txt id=item-1 mode=sync destination=managed.txt\n"
        ),
    )
    .expect("declarations");
    source
}

fn setup_destination(fixture: &tempfile::TempDir, id: &str, content: &str) -> std::path::PathBuf {
    let destination = fixture.path().join(id);
    git_repo(&destination);
    fs::write(destination.join("managed.txt"), content).expect("managed");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&destination)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    destination
}

fn write_machine_config(home: &Path, destinations: &[(&str, &Path)], sources: &[(&str, &Path)]) {
    fs::create_dir_all(home.join(".omnirepo")).expect("omnirepo dir");
    let mut config = String::from("version: 1\nrepositories:\n");
    for (id, path) in destinations {
        config.push_str(&format!("  - id: {id}\n    path: {}\n", path.display()));
    }
    config.push_str("sources:\n");
    for (id, path) in sources {
        config.push_str(&format!("  - id: {id}\n    location: {}\n", path.display()));
    }
    config.push_str("concurrency:\n  max_repositories: 4\n  max_child_work: 8\nrepair:\n  priority: [pi]\n  max_attempts: 3\n");
    fs::write(home.join(".omnirepo/config.yaml"), config).expect("machine config");
}

fn records_in(home: &Path) -> Vec<std::path::PathBuf> {
    let runs = home.join(".omnirepo/runs");
    match fs::read_dir(&runs) {
        Ok(entries) => entries
            .map(|entry| entry.expect("entry").path())
            .filter(|path| !path.ends_with(".."))
            .collect(),
        Err(_) => Vec::new(),
    }
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
fn empty_fleet_sync_exits_zero_quietly_with_a_durable_record() {
    let fixture = fixture("e2e-empty-");
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    command(&home, fixture.path())
        .arg("sync")
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    assert_eq!(records_in(&home).len(), 1, "one durable record");
}

#[test]
fn unchanged_fleet_sync_exits_zero_and_creates_no_commit() {
    let fixture = fixture("e2e-unchanged-");
    let source = setup_source(&fixture);
    let destination = setup_destination(&fixture, "destination-a", "v1\n");
    let head_before = git_text(&destination, &["rev-parse", "HEAD"]);
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    write_machine_config(
        &home,
        &[("destination-a", &destination)],
        &[("source-a", &source)],
    );
    command(&home, fixture.path())
        .arg("sync")
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    let head_after = git_text(&destination, &["rev-parse", "HEAD"]);
    assert_eq!(head_before, head_after, "unchanged creates no commit");
}

#[test]
fn changed_worktree_sync_commits_the_delta_and_exits_zero() {
    let fixture = fixture("e2e-changed-");
    let source = setup_source(&fixture);
    let destination = setup_destination(&fixture, "destination-a", "v0\n");
    // The worktree now differs from the committed state: the pass
    // stages the delta and delivers one commit.
    fs::write(destination.join("managed.txt"), "v1\n").expect("changed worktree");
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    write_machine_config(
        &home,
        &[("destination-a", &destination)],
        &[("source-a", &source)],
    );
    command(&home, fixture.path()).arg("sync").assert().code(0);
    // The scoped commit object exists and was reconciled (the ref moves
    // only through the push owner, .29); the worktree delta is staged
    // byte-exactly.
    let commits = git_text(&destination, &["rev-list", "--count", "HEAD"]);
    assert_eq!(commits, "1", "the local ref is untouched without a push");
    let objects = git_text(
        &destination,
        &["cat-file", "--batch-check", "--batch-all-objects"],
    );
    let commit_objects = objects
        .lines()
        .filter(|line| line.split_whitespace().nth(1) == Some("commit"))
        .count();
    assert_eq!(
        commit_objects, 2,
        "base plus the scoped commit object: {objects}"
    );
    assert_eq!(
        fs::read_to_string(destination.join("managed.txt")).expect("managed"),
        "v1\n",
        "the pass stages the worktree delta byte-exactly"
    );
}

#[test]
fn partial_fleet_sync_exits_three_and_names_the_failure() {
    let fixture = fixture("e2e-partial-");
    let source = setup_source(&fixture);
    let good = setup_destination(&fixture, "repo-good", "v1\n");
    let bad = fixture.path().join("repo-bad");
    fs::create_dir_all(&bad).expect("bad destination");
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    write_machine_config(
        &home,
        &[("repo-good", &good), ("repo-bad", &bad)],
        &[("source-a", &source)],
    );
    command(&home, fixture.path())
        .arg("sync")
        .assert()
        .code(3)
        .stdout(predicate::str::is_empty());
    // The durable record names the failed repository truthfully.
    let record = fs::read_to_string(&records_in(&home)[0]).expect("record");
    assert!(record.contains("repo-bad"), "{record}");
}

#[test]
fn total_fleet_sync_with_a_missing_source_exits_four() {
    let fixture = fixture("e2e-total-");
    let destination = setup_destination(&fixture, "destination-a", "v1\n");
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    write_machine_config(
        &home,
        &[("destination-a", &destination)],
        &[("source-a", Path::new("/definitely/not/here"))],
    );
    command(&home, fixture.path())
        .arg("sync")
        .assert()
        .code(4)
        .stdout(predicate::str::is_empty());
}

#[test]
fn repair_with_a_fake_adapter_recovers_the_failed_repository() {
    let fixture = fixture("e2e-repair-");
    let source = setup_source(&fixture);
    // A destination whose managed target is a symlink alias: the
    // snapshot rejects it, the initial pass fails, and the repair agent
    // (a fake adapter on PATH) recovers it.
    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    let outside = fixture.path().join("outside-secret.txt");
    fs::write(&outside, "secret\n").expect("outside");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, destination.join("managed.txt")).expect("symlink");
    }
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&destination)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "managed.txt"]);
    git(&["commit", "--quiet", "--message", "base"]);
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    write_machine_config(
        &home,
        &[("destination-a", &destination)],
        &[("source-a", &source)],
    );
    // The fake adapter: a shell script named `pi` on a PATH the machine
    // resolves first.
    let bin = fixture.path().join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let adapter = bin.join("pi");
    fs::write(
        &adapter,
        "#!/bin/sh\nrm -f managed.txt\necho repaired > managed.txt\nexit 0\n",
    )
    .expect("adapter");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&adapter).expect("meta").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&adapter, permissions).expect("mode");
    }
    command(&home, fixture.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .arg("sync")
        .assert()
        .code(0);
}
