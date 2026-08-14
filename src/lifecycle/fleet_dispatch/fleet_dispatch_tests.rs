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
    git(&source, &["add", "."]);
    git(&source, &["commit", "--quiet", "--message", "source"]);
    let source_head = git_text(&source, &["rev-parse", "HEAD"]);
    fs::write(
        source.join(".omnirepo/source.yaml"),
        format!(
            "omnirepo-declarations-v1\nsource=source-a revision={source_head} path=managed.txt id=item-1 mode=sync destination=managed.txt\n"
        ),
    )
    .expect("declarations");
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
