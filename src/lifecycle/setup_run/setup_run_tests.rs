//! Focused proof for connecting setup completion to the first inferred
//! synchronization.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::lifecycle::setup_run::{SetupRequest, SetupRunError, run_setup};
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
        .prefix("setup-run-")
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
    git(&["config", "user.name", "Setup"]);
    git(&["config", "user.email", "setup@example.test"]);
}

fn journal_fixture(home: &Path) -> (Journal, String) {
    let runs = home.join(".omnirepo/runs");
    fs::create_dir_all(&runs).expect("runs");
    let record = RunRecord::create_with_id(
        home,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (journal, run_id)
}

#[test]
fn plan_display_without_apply_writes_nothing() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        false,
        false,
    );
    let outcome = run_setup(&home, &request, None).expect("plan");
    assert!(
        outcome.applied.is_empty(),
        "nothing was applied: {:?}",
        outcome
    );
    assert!(
        !home.join(".omnirepo/config.yaml").exists(),
        "no file written"
    );
}

#[test]
fn apply_with_confirmation_authors_the_config_and_repeat_is_a_no_op() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        true,
        true,
    );
    let outcome = run_setup(&home, &request, None).expect("apply");
    assert_eq!(outcome.applied.len(), 1, "{:?}", outcome);
    assert_eq!(
        fs::read_to_string(home.join(".omnirepo/config.yaml")).expect("config"),
        "version: 1\nrepositories: []\n"
    );
    let second = run_setup(&home, &request, None).expect("repeat");
    assert!(second.applied.is_empty(), "repeated apply is a no-op");
}

#[test]
fn apply_without_confirmation_is_refused() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        true,
        false,
    );
    let error = run_setup(&home, &request, None).expect_err("no confirmation");
    assert!(
        matches!(error, SetupRunError::ConfirmationRequired),
        "{error}"
    );
    assert!(!home.join(".omnirepo/config.yaml").exists());
}

#[test]
fn an_invalid_existing_authority_is_never_replaced() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(home.join(".omnirepo")).expect("dir");
    fs::write(home.join(".omnirepo/config.yaml"), "bogus: [x\n").expect("invalid");
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        true,
        true,
    );
    let error = run_setup(&home, &request, None).expect_err("refused");
    assert!(
        matches!(error, SetupRunError::ConflictingAuthority { .. }),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(home.join(".omnirepo/config.yaml")).expect("untouched"),
        "bogus: [x\n"
    );
}
