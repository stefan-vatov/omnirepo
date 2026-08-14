//! Focused proof for wiring cancellation into the CLI.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::exit_status::{ExitClass, classify_summary};
use crate::lifecycle::fleet_cancel::{CancelOutcome, cancel_fleet_run};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-cancel-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let fixture = fixture_base();
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

fn record_lines(path: &std::path::Path) -> String {
    fs::read_to_string(path).expect("record")
}

#[test]
fn cancellation_classifies_every_selected_repository() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let fleet = vec![
        "repo-a".to_owned(),
        "repo-b".to_owned(),
        "repo-c".to_owned(),
    ];
    let outcome = cancel_fleet_run(&journal.handle, &run_id, &fleet).expect("cancel");
    assert_eq!(outcome.exit_class, ExitClass::Cancelled);
    journal.shutdown().expect("shutdown");
    let record = record_lines(&record_path);
    for repository in &fleet {
        assert!(
            record.contains(repository),
            "the cancelled classification names {repository}"
        );
    }
    assert!(record.contains("cancelled"), "{record}");
    assert!(record.contains("\"type\":\"cancelled\""), "{record}");
}

#[test]
fn cancellation_is_exit_130_and_finalizes_the_record() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let outcome =
        cancel_fleet_run(&journal.handle, &run_id, &["repo-a".to_owned()]).expect("cancel");
    assert_eq!(outcome.exit_class, ExitClass::Cancelled);
    journal.shutdown().expect("shutdown");
    let record = record_lines(&record_path);
    assert!(record.contains("\"outcome\":\"cancelled\""), "{record}");
}

#[test]
fn an_empty_fleet_cancellation_still_finalizes_cancelled() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let outcome = cancel_fleet_run(&journal.handle, &run_id, &[]).expect("cancel");
    assert_eq!(outcome.exit_class, ExitClass::Cancelled);
    journal.shutdown().expect("shutdown");
    let record = record_lines(&record_path);
    assert!(record.contains("\"type\":\"cancelled\""), "{record}");
}

#[test]
fn the_cancelled_summary_classifies_as_130() {
    use crate::lifecycle::run_summary::{
        RepoEntry, RepoOutcome, RunSummary, SummaryStatus, fold_summary,
    };
    let summary = fold_summary(
        "run-1",
        vec![
            (
                "repo-a".to_owned(),
                RepoOutcome::Cancelled,
                "evidence-1".to_owned(),
            ),
            (
                "repo-b".to_owned(),
                RepoOutcome::Cancelled,
                "evidence-2".to_owned(),
            ),
        ],
        true,
    )
    .expect("summary");
    assert_eq!(summary.status, SummaryStatus::Cancelled);
    assert_eq!(classify_summary(&summary, true), ExitClass::Cancelled);
}
