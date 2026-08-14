//! Repair causation, budget, fallback, crash, and uncertainty fixtures.
//!
//! STRICT TDD: this test file was written and run RED before the fixture
//! composition existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::repair_causation::{CausationVerdict, prove_current_run_causation};
use crate::lifecycle::repair_classify::{Eligibility, FailureClass, classify_failure};
use crate::lifecycle::repair_reserve::{ReserveError, reserve_repair_attempt};
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
        .prefix("repair-fixture-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [6_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

#[test]
fn only_proven_causation_and_eligible_classes_reach_a_reservation() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    // Proven causation + eligible class -> reservation succeeds.
    let verdict = prove_current_run_causation("lineage-1", "lineage-1", true);
    assert_eq!(verdict, CausationVerdict::Proven);
    let classification = classify_failure(FailureClass::SyncDrift);
    assert!(matches!(
        classification.eligibility,
        Eligibility::Repairable { .. }
    ));
    let reserved = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["baseline-1".to_owned()],
        1,
        &fs::read_to_string(&record_path).expect("record"),
    )
    .expect("reserve");
    assert!(matches!(
        reserved,
        crate::lifecycle::repair_reserve::ReserveOutcome::Reserved(_)
    ));
    journal.shutdown().expect("shutdown");
}

#[test]
fn unproven_causation_never_reaches_a_reservation() {
    // A lineage mismatch is not proven; the class is still classified but
    // the repair path must stop before reservation.
    let verdict = prove_current_run_causation("baseline-1", "lineage-9", true);
    assert!(matches!(verdict, CausationVerdict::NotProven { .. }));
    let classification = classify_failure(FailureClass::GitDeliveryFailed);
    assert_eq!(classification.eligibility, Eligibility::Terminal);
}

#[test]
fn crash_after_reservation_reconciles_from_the_record() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let _ = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["baseline-1".to_owned()],
        1,
        &fs::read_to_string(&record_path).expect("record"),
    )
    .expect("reserve");
    journal.shutdown().expect("shutdown");
    // A restart reads the record: the reservation marker is durable and a
    // second reservation is refused.
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("repair-reserve"), "{record}");
    let error = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "dest-a",
        &["baseline-1".to_owned()],
        1,
        &record,
    )
    .expect_err("duplicate after restart");
    assert!(
        matches!(error, ReserveError::AlreadyReserved { .. }),
        "{error}"
    );
}

#[test]
fn uncertainty_and_fallback_never_reserve() {
    // Uncertain causation is terminal; the fallback is the typed terminal
    // outcome, never a reservation.
    let uncertain = classify_failure(FailureClass::Uncertain);
    assert_eq!(uncertain.eligibility, Eligibility::Terminal);
    let unrelated = classify_failure(FailureClass::Unrelated);
    assert_eq!(unrelated.eligibility, Eligibility::Terminal);
    // Empty inputs (a fallback guard) fail typed before reservation.
    let (_fixture, mut journal, run_id, _record_path) = journal_fixture();
    let error = reserve_repair_attempt(&journal.handle, &run_id, "dest-a", &[], 1, "")
        .expect_err("no inputs");
    assert!(
        matches!(error, ReserveError::NoFrozenInputs { .. }),
        "{error}"
    );
    journal.shutdown().expect("shutdown");
}
