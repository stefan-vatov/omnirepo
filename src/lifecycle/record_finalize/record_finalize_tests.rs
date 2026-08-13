//! Focused proof for atomic record finalization.

#![allow(dead_code, unused_imports)]

use super::{FinalizeError, FinalizeOutcome, finalize_path, terminal_marker};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn record_fixture() -> (tempfile::TempDir, RunRecord, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("finalize-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [8_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    (fixture, record, run_id)
}

#[test]
fn completed_records_replay_as_terminal() {
    let (_fixture, record, run_id) = record_fixture();
    let outcome = finalize_path(record.path(), &run_id).expect("finalize");
    assert_eq!(outcome, FinalizeOutcome::Finalized);
    let content = fs::read_to_string(record.path()).expect("record");
    assert!(content.contains("\"type\":\"terminal\""), "{content}");
    // The record still parses as a canonical event stream (no false
    // path or reference is emitted).
    assert!(
        content
            .lines()
            .last()
            .expect("last line")
            .starts_with("{\"version\":1"),
        "{content}"
    );
}

#[test]
fn finalization_is_idempotent() {
    let (_fixture, record, run_id) = record_fixture();
    assert_eq!(
        finalize_path(record.path(), &run_id).expect("finalize"),
        FinalizeOutcome::Finalized
    );
    assert_eq!(
        finalize_path(record.path(), &run_id).expect("finalize again"),
        FinalizeOutcome::AlreadyFinalized
    );
    // The marker appears exactly once.
    let content = fs::read_to_string(record.path()).expect("record");
    assert_eq!(
        content.matches("\"type\":\"terminal\"").count(),
        1,
        "{content}"
    );
}

#[test]
fn missing_records_fail_typed_without_false_output() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let missing = base.join("finalize-missing-record.jsonl");
    let _ = fs::remove_file(&missing);
    let error = finalize_path(&missing, "run-missing").expect_err("missing");
    assert!(matches!(error, FinalizeError::Read { .. }), "{error}");
}

#[test]
fn terminal_marker_is_canonical_and_versioned() {
    let marker = terminal_marker("run-1");
    assert!(marker.starts_with("{\"version\":1"), "{marker}");
    assert!(marker.contains("\"type\":\"terminal\""), "{marker}");
    assert!(marker.contains("\"outcome\":\"success\""), "{marker}");
}
