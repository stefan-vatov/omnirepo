//! Focused proof for cancellation and terminalization.

use super::{cancel_run, terminalize_in_flight, terminalize_not_started};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("cancellation-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [4_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

#[test]
fn cancelled_run_records_queued_and_in_flight_repositories() {
    let (_fixture, mut journal, run_id, record_path) = fixture();
    cancel_run(
        &journal.handle,
        &run_id,
        &["dest-a".to_owned(), "dest-b".to_owned()],
    )
    .expect("cancel");
    journal.shutdown().expect("shutdown");
    let path = &record_path;
    let content = fs::read_to_string(path).expect("events");
    assert!(content.contains("\"outcome\":\"cancelled\""), "{content}");
    assert!(
        content.contains("\"repository_id\":\"dest-a\""),
        "{content}"
    );
    assert!(
        content.contains("\"repository_id\":\"dest-b\""),
        "{content}"
    );
    assert!(content.contains("\"stage\":\"cancellation\""), "{content}");
}

#[test]
fn empty_fleet_cancellation_fails_typed() {
    let (_fixture, journal, run_id, _record_path) = fixture();
    let error = cancel_run(&journal.handle, &run_id, &[]).expect_err("empty");
    assert!(format!("{error}").contains("no repositories"), "{error}");
}

#[test]
fn not_started_and_in_flight_terminalize_as_cancelled() {
    let (_fixture, mut journal, run_id, record_path) = fixture();
    terminalize_not_started(&journal.handle, &run_id).expect("not started");
    journal.shutdown().expect("shutdown");
    let path = &record_path;
    let content = fs::read_to_string(path).expect("events");
    assert!(content.contains("\"type\":\"cancelled\""), "{content}");

    let (_fixture2, mut journal2, run_id2, record_path2) = fixture();
    terminalize_in_flight(&journal2.handle, &run_id2, &["dest-a".to_owned()]).expect("in flight");
    journal2.shutdown().expect("shutdown");
    let content2 = fs::read_to_string(record_path2).expect("events");
    assert!(
        content2.contains("\"repository_id\":\"dest-a\""),
        "{content2}"
    );
    assert!(content2.contains("\"type\":\"cancelled\""), "{content2}");
}
