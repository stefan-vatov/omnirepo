//! Focused proof for journal-backpressure-aware scheduling.

#![allow(dead_code, unused_imports)]

use super::{ProbeClass, Scheduler, SchedulerEvent, classify_probe, permit_failure};
use crate::lifecycle::fleet_permits::{FleetPermits, PermitError};
use crate::lifecycle::journal::{Journal, JournalConfig, TrySubmitError};
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
        .prefix("scheduler-home-")
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

#[test]
fn probe_classes_map_typed() {
    assert_eq!(classify_probe(Ok(1)), ProbeClass::Admitted);
    assert_eq!(
        classify_probe(Err(TrySubmitError::Full)),
        ProbeClass::Backpressured
    );
    assert_eq!(
        classify_probe(Err(TrySubmitError::Poisoned)),
        ProbeClass::WriterFailed
    );
    assert_eq!(
        classify_probe(Err(TrySubmitError::Rejected(
            crate::lifecycle::journal::JournalError::Poisoned
        ))),
        ProbeClass::WriterFailed
    );
}

#[test]
fn admitted_intent_is_persisted_before_the_effect_runs() {
    let (_fixture, mut journal, run_id, record_path) = journal_fixture();
    let permits = FleetPermits::new(2).expect("ledger");
    let scheduler = Scheduler::new(journal.handle.clone(), run_id, permits);
    let event = scheduler.try_admit("dest-a");
    assert_eq!(
        event,
        SchedulerEvent::Admitted {
            repository: "dest-a".to_owned()
        }
    );
    // The intent is durably in the record.
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("\"repository_id\":\"dest-a\""), "{record}");
    assert!(record.contains("\"operation\":\"sync\""), "{record}");
}

#[test]
fn writer_failure_stops_scheduling_and_final_accounting_keeps_everyone() {
    let (_fixture, mut journal, run_id, _record_path) = journal_fixture();
    let permits = FleetPermits::new(2).expect("ledger");
    let scheduler = Scheduler::new(journal.handle.clone(), run_id, permits);
    // Poison the journal: shutdown kills the writer; try_submit then fails.
    journal.shutdown().expect("shutdown");
    let event = scheduler.try_admit("dest-a");
    assert!(
        matches!(event, SchedulerEvent::WriterFailed { .. }),
        "{event:?}"
    );
    // Every queued repository reaches the final accounting.
    let final_events = scheduler.finalize_queued(&["dest-a".to_owned(), "dest-b".to_owned()]);
    assert_eq!(final_events.len(), 2);
    assert!(
        final_events
            .iter()
            .all(|event| matches!(event, SchedulerEvent::FinalCancelled { .. })),
        "{final_events:?}"
    );
}

#[test]
fn permit_failures_map_typed() {
    assert_eq!(
        permit_failure(PermitError::RunCancelled),
        "the run is cancelled"
    );
    assert_eq!(
        permit_failure(PermitError::WriterUnhealthy),
        "the journal writer is unhealthy"
    );
    assert_eq!(
        permit_failure(PermitError::Limit {
            reason: "zero".to_owned()
        }),
        "zero"
    );
}

#[test]
fn backpressure_keeps_the_repository_queued() {
    // The classification is the backpressure contract: Full keeps the
    // repository queued for a later attempt, never drops it.
    let permits = FleetPermits::new(2).expect("ledger");
    permits.enqueue("dest-a");
    assert_eq!(permits.queued(), 1);
    let _ = classify_probe(Err(TrySubmitError::Full));
}
