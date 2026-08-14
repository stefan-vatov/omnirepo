//! Focused proof for finalizing the run: summary, terminal record,
//! projection, and exit.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::exit_status::ExitClass;
use crate::lifecycle::fleet_app::FleetResponse;
use crate::lifecycle::fleet_collector::MemberResult;
use crate::lifecycle::fleet_finalize::finalize_fleet_run;
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
        .prefix("fleet-finalize-")
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

fn response(run_id: &str, results: Vec<MemberResult>) -> FleetResponse {
    FleetResponse {
        run_id: run_id.to_owned(),
        results,
        frozen_repair_inputs: Vec::new(),
    }
}

#[test]
fn a_delivered_fleet_finalizes_as_success_with_quiet_projection() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let response = response(
        &run_id,
        vec![
            MemberResult::Delivered {
                repository: "repo-a".to_owned(),
                oid: "abc".to_owned(),
            },
            MemberResult::Delivered {
                repository: "repo-b".to_owned(),
                oid: "def".to_owned(),
            },
        ],
    );
    let outcome = finalize_fleet_run(&journal.handle, &run_id, &response, &[]).expect("finalize");
    assert_eq!(outcome.exit_class, ExitClass::Success);
    assert_eq!(outcome.summary.repositories.len(), 2);
    assert!(
        outcome.projection.contains("sync complete"),
        "{}",
        outcome.projection
    );
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("\"outcome\":\"success\""), "{record}");
}

#[test]
fn a_partial_fleet_finalizes_as_partial_with_typed_failures() {
    let (_fixture, mut journal, run_id, _record_path) = journal_fixture();
    let response = response(
        &run_id,
        vec![
            MemberResult::Delivered {
                repository: "repo-a".to_owned(),
                oid: "abc".to_owned(),
            },
            MemberResult::Failed {
                repository: "repo-b".to_owned(),
                reason: "verifier crashed".to_owned(),
            },
        ],
    );
    let outcome = finalize_fleet_run(&journal.handle, &run_id, &response, &[]).expect("finalize");
    assert_eq!(outcome.exit_class, ExitClass::PartialFleet);
    assert!(
        outcome.projection.contains("sync failed") && outcome.projection.contains("repo-b"),
        "{}",
        outcome.projection
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_total_failure_and_affected_repositories_finalize_as_failure() {
    let (_fixture, mut journal, run_id, _record_path) = journal_fixture();
    let response = response(
        &run_id,
        vec![
            MemberResult::Failed {
                repository: "repo-a".to_owned(),
                reason: "verifier crashed".to_owned(),
            },
            MemberResult::Skipped {
                repository: "repo-b".to_owned(),
                reason: "preflight denied".to_owned(),
            },
        ],
    );
    let outcome = finalize_fleet_run(
        &journal.handle,
        &run_id,
        &response,
        &["repo-c: source unavailable".to_owned()],
    )
    .expect("finalize");
    assert_eq!(outcome.exit_class, ExitClass::TotalFailure);
    assert_eq!(outcome.summary.repositories.len(), 3, "affected folds in");
    journal.shutdown().expect("shutdown");
}

#[test]
fn an_empty_fleet_finalizes_as_success() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let response = response(&run_id, Vec::new());
    let outcome = finalize_fleet_run(&journal.handle, &run_id, &response, &[]).expect("finalize");
    assert_eq!(outcome.exit_class, ExitClass::Success);
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("\"outcome\":\"success\""), "{record}");
}
