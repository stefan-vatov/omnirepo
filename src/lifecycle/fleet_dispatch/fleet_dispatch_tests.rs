//! Focused proof for the dispatch seam: machine config to fleet run to
//! exit.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::exit_status::ExitClass;
use crate::lifecycle::fleet_dispatch::{DispatchError, dispatch_fleet};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-dispatch-")
        .tempdir_in(&base)
        .expect("fixture")
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
    git(&["config", "user.name", "Dispatch"]);
    git(&["config", "user.email", "dispatch@example.test"]);
}

fn journal_fixture(home: &Path) -> (Journal, String, std::path::PathBuf) {
    let runs = home.join(".omnirepo/runs");
    fs::create_dir_all(&runs).expect("runs");
    let record = RunRecord::create_with_id(
        home,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (journal, run_id, record_path)
}

#[test]
fn an_absent_machine_authority_is_the_empty_fleet_success() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let (journal, run_id, record_path) = journal_fixture(&home);
    let outcome = dispatch_fleet(&journal.handle, &run_id, &home, &record_path).expect("dispatch");
    assert_eq!(outcome.exit_class, ExitClass::Success);
    assert_eq!(outcome.repositories, 0);
    let mut journal = journal;
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_configured_fleet_runs_end_to_end_and_finalizes_success() {
    let fixture = fixture_base();
    // The machine config: one local source + one destination.
    let source = fixture.path().join("source-a");
    git_repo(&source);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(source.join("managed.txt"), "v1\n").expect("source file");
    let git = |root: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    fs::write(
        source.join(".omnirepo/source.yaml"),
        "omnirepo-declarations-v1\nsource=source-a path=managed.txt id=item-1 mode=sync destination=managed.txt\n",
    )
    .expect("declarations");
    git(&source, &["add", "."]);
    git(&source, &["commit", "--quiet", "--message", "source"]);
    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    fs::write(destination.join("managed.txt"), "v0\n").expect("destination file");
    git(&destination, &["add", "."]);
    git(&destination, &["commit", "--quiet", "--message", "base"]);
    let home = fixture.path().join("home");
    fs::create_dir_all(home.join(".omnirepo")).expect("omnirepo dir");
    let config = format!(
        "version: 1\nrepositories:\n  - id: destination-a\n    path: {}\nsources:\n  - id: source-a\n    location: {}\nconcurrency:\n  max_repositories: 2\n  max_child_work: 4\n",
        destination.display(),
        source.display()
    );
    fs::write(home.join(".omnirepo/config.yaml"), config).expect("machine config");
    let (journal, run_id, record_path) = journal_fixture(&home);
    let outcome = dispatch_fleet(&journal.handle, &run_id, &home, &record_path).expect("dispatch");
    assert_eq!(
        outcome.exit_class,
        ExitClass::Success,
        "the fleet run succeeds"
    );
    assert_eq!(outcome.repositories, 1);
    let mut journal = journal;
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("\"outcome\":\"success\""), "{record}");
}

#[test]
fn an_unavailable_higher_priority_source_marks_the_repository_affected() {
    let fixture = fixture_base();
    // The lower-priority source is complete and declares the managed item.
    let source = fixture.path().join("source-b");
    git_repo(&source);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(source.join("managed.txt"), "v1\n").expect("source file");
    let git = |root: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    fs::write(
        source.join(".omnirepo/source.yaml"),
        "omnirepo-declarations-v1\nsource=source-b path=managed.txt id=item-1 mode=sync destination=managed.txt\n",
    )
    .expect("declarations");
    git(&source, &["add", "."]);
    git(&source, &["commit", "--quiet", "--message", "source"]);
    // A valid destination that would otherwise sync successfully.
    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    fs::write(destination.join("managed.txt"), "v0\n").expect("destination file");
    git(&destination, &["add", "."]);
    git(&destination, &["commit", "--quiet", "--message", "base"]);
    // The HIGHER-priority source (configured first) is unavailable.
    let home = fixture.path().join("home");
    fs::create_dir_all(home.join(".omnirepo")).expect("omnirepo dir");
    let config = format!(
        "version: 1\nrepositories:\n  - id: destination-a\n    path: {}\nsources:\n  - id: source-a\n    location: {}\n  - id: source-b\n    location: {}\n",
        destination.display(),
        "/definitely/not/here",
        source.display()
    );
    fs::write(home.join(".omnirepo/config.yaml"), config).expect("machine config");
    let (journal, run_id, record_path) = journal_fixture(&home);
    let outcome = dispatch_fleet(&journal.handle, &run_id, &home, &record_path).expect("dispatch");
    // The lower source must not be silently promoted into the unavailable
    // higher source's authority: the repository is affected, and the
    // destination content stays untouched.
    assert_eq!(
        outcome.exit_class,
        ExitClass::TotalFailure,
        "the affected repository fails instead of silently syncing"
    );
    assert_eq!(
        fs::read_to_string(destination.join("managed.txt")).expect("destination"),
        "v0\n",
        "no lower-source content is applied"
    );
    let mut journal = journal;
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_missing_source_authority_fails_the_fleet_typed() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(home.join(".omnirepo")).expect("omnirepo dir");
    let config = format!(
        "version: 1\nrepositories:\n  - id: destination-a\n    path: {}\nsources:\n  - id: source-a\n    location: {}\n",
        fixture.path().join("destination-a").display(),
        "/definitely/not/here"
    );
    fs::write(home.join(".omnirepo/config.yaml"), config).expect("machine config");
    let (journal, run_id, record_path) = journal_fixture(&home);
    let outcome = dispatch_fleet(&journal.handle, &run_id, &home, &record_path);
    assert!(outcome.is_ok(), "the dispatch never panics: {outcome:?}");
    let outcome = outcome.expect("dispatch");
    assert_eq!(
        outcome.exit_class,
        ExitClass::TotalFailure,
        "the unavailable source affects the only repository"
    );
    let mut journal = journal;
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_remote_source_runs_the_complete_fleet_pipeline() {
    let fixture = fixture_base();
    let source = fixture.path().join("source-work");
    git_repo(&source);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(source.join("managed.txt"), "remote-v1\n").expect("source file");
    fs::write(
        source.join(".omnirepo/source.yaml"),
        "omnirepo-declarations-v1\nsource=source-a path=managed.txt id=item-1 mode=sync destination=managed.txt\n",
    )
    .expect("declarations");
    let git = |root: &Path, args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    };
    git(&source, &["add", "."]);
    git(&source, &["commit", "--quiet", "--message", "source"]);
    let remote = fixture.path().join("source.git");
    let output = Command::new("git")
        .args(["init", "--quiet", "--bare", "-b", "main"])
        .arg(&remote)
        .output()
        .expect("bare remote");
    assert!(output.status.success(), "bare remote: {output:?}");
    let remote_url = format!("file://{}", remote.display());
    git(&source, &["push", "--quiet", &remote_url, "main"]);

    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    fs::write(destination.join("managed.txt"), "v0\n").expect("destination file");
    git(&destination, &["add", "."]);
    git(&destination, &["commit", "--quiet", "--message", "base"]);
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("cache");
    let config = crate::configuration::MachineConfiguration::new(
        crate::configuration::SchemaVersion::new(1).expect("version"),
        vec![
            crate::configuration::DestinationRepository::new(
                crate::configuration::RepositoryId::parse("destination-a").expect("id"),
                crate::configuration::AbsolutePath::parse(
                    destination.to_str().expect("destination utf8"),
                )
                .expect("destination path"),
                Vec::new(),
            )
            .expect("destination"),
        ],
        vec![crate::configuration::SourceReference::new(
            crate::configuration::SourceId::parse("source-a").expect("id"),
            crate::configuration::SourceLocation::Remote(remote_url),
        )],
        Some(
            crate::configuration::AbsolutePath::parse(cache.to_str().expect("cache utf8"))
                .expect("cache path"),
        ),
        crate::configuration::MachineConcurrency::new(2, 4).expect("concurrency"),
        crate::configuration::RepairControls::default(),
    )
    .expect("machine config");
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let (journal, run_id, record_path) = journal_fixture(&home);

    let outcome = super::run_configured(&journal.handle, &run_id, &config, &record_path)
        .expect("configured fleet");

    assert_eq!(outcome.exit_class, ExitClass::Success, "{outcome:?}");
    assert_eq!(
        fs::read_to_string(destination.join("managed.txt")).expect("managed destination"),
        "remote-v1\n"
    );
    let mut journal = journal;
    journal.shutdown().expect("shutdown");
}
