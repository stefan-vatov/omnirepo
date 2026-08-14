//! Focused proof for freezing repair inputs and durably reserving exactly
//! one attempt.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::repair_reserve::{ReserveError, ReserveOutcome, reserve_repair_attempt};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn journal_fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-reserve-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [5_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

fn record_content(path: &std::path::Path) -> String {
    fs::read_to_string(path).expect("record")
}

#[test]
fn exactly_one_attempt_is_reserved_and_durably_journaled() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let outcome = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["baseline-1".to_owned(), "delta-1".to_owned()],
        1,
        &record_content(&record_path),
    )
    .expect("reserve");
    let ReserveOutcome::Reserved(reservation) = outcome;
    assert_eq!(reservation.attempt, 1);
    assert!(reservation.journaled);
    assert!(!reservation.reservation_id.is_empty());
    // A second reservation for the same repository is refused: exactly one
    // attempt is reserved.
    let error = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["baseline-1".to_owned(), "delta-1".to_owned()],
        1,
        &record_content(&record_path),
    )
    .expect_err("already reserved");
    assert!(
        matches!(error, ReserveError::AlreadyReserved { .. }),
        "{error}"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn frozen_inputs_are_deduplicated_and_non_empty() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let outcome = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["x".to_owned(), "x".to_owned(), "y".to_owned()],
        2,
        &record_content(&record_path),
    )
    .expect("reserve");
    let ReserveOutcome::Reserved(reservation) = outcome;
    assert_eq!(
        reservation.frozen_inputs,
        vec!["x".to_owned(), "y".to_owned()]
    );
    // Empty inputs fail typed.
    let error = reserve_repair_attempt(&journal.handle, &run_id, "dest-b", &[], 2, "")
        .expect_err("empty inputs");
    assert!(
        matches!(error, ReserveError::NoFrozenInputs { .. }),
        "{error}"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn attempts_beyond_the_budget_are_exhausted() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let first = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["x".to_owned()],
        1,
        &record_content(&record_path),
    )
    .expect("first");
    let ReserveOutcome::Reserved(first_reservation) = first;
    // The reservation is durable: a second reserve for the same repository
    // is still refused within the same run (one durable reservation per
    // repository per run).
    let _ = first_reservation;
    let error = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["x".to_owned()],
        1,
        &record_content(&record_path),
    )
    .expect_err("already reserved");
    assert!(
        matches!(error, ReserveError::AlreadyReserved { .. }),
        "{error}"
    );
    journal.shutdown().expect("shutdown");
}
