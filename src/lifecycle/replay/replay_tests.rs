//! Focused proof for deterministic journal replay and incomplete-run discovery.

#![allow(dead_code, unused_imports)]

use super::{MAX_REPLAY_BYTES, ReplayError, TailStatus, discover_incomplete, replay};
use crate::lifecycle::event::{JournalEvent, Operation, Outcome, RunStage};
use crate::lifecycle::run_record::RunRecord;
use std::{fs, path::Path};

fn run_id() -> String {
    "2026-08-13T16:00:00.000000000Z-a1b2c3d4e5f60718a1b2c3d4e5f60718".to_owned()
}

fn fixture_record() -> (tempfile::TempDir, RunRecord, std::path::PathBuf, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let home = tempfile::Builder::new()
        .prefix("replay-home-")
        .tempdir_in(&base)
        .expect("create filesystem fixture home");
    fs::create_dir_all(home.path().join(".omnirepo/runs")).expect("create run-record parent");
    let record = RunRecord::create_with_id(
        home.path(),
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        [8_u8; 16],
    )
    .expect("create fixture record");
    let record_path = record.path().to_path_buf();
    let actual_run_id = record.id().to_string();
    (home, record, record_path, actual_run_id)
}

fn intent(checkpoint: u64) -> JournalEvent {
    JournalEvent::RunIntent {
        checkpoint,
        run_id: run_id(),
        stage: RunStage::Invocation,
    }
}

fn terminal(checkpoint: u64, outcome: Outcome) -> JournalEvent {
    JournalEvent::Terminal {
        checkpoint,
        run_id: run_id(),
        outcome,
    }
}

#[test]
fn complete_record_replays_all_events_and_is_complete() {
    let (_home, mut record, record_path, actual_run_id) = fixture_record();
    // The record already contains the invocation intent (checkpoint 0); a
    // terminal completes it.
    let terminal = terminal(1, Outcome::Success).render();
    record.append(terminal.as_bytes()).expect("append terminal");
    drop(record);
    let result = replay(&record_path).expect("replay");
    assert!(result.complete);
    assert_eq!(result.tail, TailStatus::Clean);
    assert_eq!(result.events.len(), 2);
    assert_eq!(result.events[0].run_id(), actual_run_id);
    assert_eq!(result.events[0].checkpoint(), 0);
    assert_eq!(result.events[1].checkpoint(), 1);
}

#[test]
fn truncated_tail_replays_the_valid_prefix_as_incomplete() {
    let (_home, mut record, record_path, _actual_run_id) = fixture_record();
    // Write a partial line without a trailing newline after the intent.
    record
        .append(b"{\"version\":1,\"checkpoint\":1,\"run_id\":\"partial")
        .expect("append truncated tail");
    drop(record);
    let result = replay(&record_path).expect("replay");
    assert!(!result.complete);
    assert_eq!(result.events.len(), 1, "only the valid prefix replays");
    match result.tail {
        TailStatus::Truncated { line: 2 } => {}
        other => panic!("expected truncated tail, got {other:?}"),
    }
}

#[test]
fn corrupt_tail_is_typed_and_never_claims_success() {
    let (_home, mut record, record_path, _actual_run_id) = fixture_record();
    record
        .append(b"{\"version\":1,\"checkpoint\":1,\"run_id\":\"x\",\"type\":\"nonsense\"}\n")
        .expect("append corrupt line");
    drop(record);
    let result = replay(&record_path).expect("replay");
    assert!(!result.complete);
    assert_eq!(result.events.len(), 1);
    match result.tail {
        TailStatus::Corrupt { line: 2, reason } => {
            assert!(reason.contains("unknown"), "reason: {reason}");
        }
        other => panic!("expected corrupt tail, got {other:?}"),
    }
}

#[test]
fn unsupported_version_tail_is_typed() {
    let (_home, mut record, record_path, _actual_run_id) = fixture_record();
    record
        .append(
            b"{\"version\":99,\"checkpoint\":1,\"type\":\"terminal\",\"outcome\":\"success\"}\n",
        )
        .expect("append future-version line");
    drop(record);
    let result = replay(&record_path).expect("replay");
    assert!(!result.complete);
    match result.tail {
        TailStatus::UnsupportedVersion {
            line: 2,
            version: 99,
        } => {}
        other => panic!("expected unsupported version, got {other:?}"),
    }
}

#[test]
fn transition_violation_in_tail_is_typed_corruption() {
    let (_home, mut record, record_path, _actual_run_id) = fixture_record();
    // A terminal followed by a run intent violates the transition rules; the
    // intent line replays, the violation is typed.
    let terminal = terminal(1, Outcome::Success).render();
    record.append(terminal.as_bytes()).expect("append terminal");
    let violation = intent(2).render();
    record
        .append(violation.as_bytes())
        .expect("append violating line");
    drop(record);
    let result = replay(&record_path).expect("replay");
    assert!(!result.complete);
    assert_eq!(result.events.len(), 2);
    match result.tail {
        TailStatus::Corrupt { line: 3, reason } => {
            assert!(
                reason.contains("invalid journal transition"),
                "reason: {reason}"
            );
        }
        other => panic!("expected corrupt tail, got {other:?}"),
    }
}

#[test]
fn replay_is_deterministic_and_repeated_reads_are_identical() {
    let (_home, mut record, record_path, _actual_run_id) = fixture_record();
    let terminal = terminal(1, Outcome::Success).render();
    record.append(terminal.as_bytes()).expect("append terminal");
    drop(record);
    let first = replay(&record_path).expect("first replay");
    let second = replay(&record_path).expect("second replay");
    assert_eq!(first, second, "replay must be deterministic");
}

#[test]
fn discovery_lists_only_incomplete_records_and_ignores_unrelated_files() {
    let (home, mut record, _, _) = fixture_record();
    let intent = intent(1).render();
    record
        .append(intent.as_bytes())
        .expect("append intent event");
    drop(record);
    // A complete sibling record plus unrelated files must not be listed.
    let runs = home.path().join(".omnirepo/runs");
    let complete = RunRecord::create_with_id(
        home.path(),
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_001),
        [9_u8; 16],
    )
    .expect("complete sibling");
    let complete_path = complete.path().to_path_buf();
    drop(complete);
    fs::write(&complete_path, "{\"version\":1,\"checkpoint\":0,\"run_id\":\"x\",\"type\":\"run_intent\",\"stage\":\"invocation\",\"status\":\"started\"}\n{\"version\":1,\"checkpoint\":1,\"run_id\":\"x\",\"type\":\"terminal\",\"outcome\":\"success\"}\n")
        .expect("write complete sibling");
    fs::write(runs.join("notes.txt"), "unrelated").expect("write unrelated file");
    fs::create_dir_all(runs.join("nested")).expect("create nested dir");
    fs::write(runs.join("nested/deep.log"), "ignored").expect("write nested file");

    let incomplete = discover_incomplete(&runs).expect("discover");
    assert_eq!(
        incomplete.len(),
        1,
        "only the incomplete record: {incomplete:?}"
    );
    assert!(
        incomplete[0].to_string_lossy().ends_with(".log"),
        "listed path must be a record: {:?}",
        incomplete[0]
    );
    assert!(!incomplete[0].to_string_lossy().ends_with("nested/deep.log"));
}

#[test]
fn missing_record_is_a_typed_read_error() {
    let error = replay(Path::new("/nonexistent-record-xyz.log")).expect_err("missing record");
    assert!(matches!(error, ReplayError::Read { .. }));
}

#[test]
fn replay_never_reads_beyond_the_bounded_size() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let huge = base.join("replay-huge.log");
    // A sparse file larger than the bound must be rejected before reading.
    let file = fs::File::create(&huge).expect("create sparse file");
    file.set_len(MAX_REPLAY_BYTES + 1)
        .expect("extend sparse file");
    drop(file);
    let error = replay(&huge).expect_err("oversized record");
    let _ = fs::remove_file(&huge);
    assert!(matches!(error, ReplayError::Read { .. }));
}
