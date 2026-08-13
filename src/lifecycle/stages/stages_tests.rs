//! Focused proof for run stage checkpoints and frozen-input revalidation.

#![allow(dead_code, unused_imports)]

use super::{RunStage, StageError, StageMachine};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{fs, path::Path, time::Duration, time::SystemTime};

fn fixture_journal() -> (tempfile::TempDir, Journal, std::path::PathBuf, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("stages-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [2_u8; 16],
    )
    .expect("record");
    let path = record.path().to_path_buf();
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, path, run_id)
}

#[test]
fn stage_machine_enforces_declared_order_and_checkpoints() {
    let (_fixture, mut journal, record_path, run_id) = fixture_journal();
    let mut machine = StageMachine::new();
    // Jumps and starts out of order fail.
    let error = machine
        .advance(&journal.handle, &run_id, RunStage::Admission)
        .expect_err("start must be preflight");
    assert!(
        matches!(error, StageError::InvalidTransition { .. }),
        "{error:?}"
    );
    // The declared sequence checkpoints each stage.
    for stage in [
        RunStage::Preflight,
        RunStage::Admission,
        RunStage::Synchronization,
        RunStage::Verification,
        RunStage::GitDelivery,
        RunStage::Finalization,
    ] {
        machine
            .advance(&journal.handle, &run_id, stage)
            .expect("advance");
    }
    // A repeat after finalization fails.
    let error = machine
        .advance(&journal.handle, &run_id, RunStage::Preflight)
        .expect_err("regression must fail");
    assert!(
        matches!(error, StageError::InvalidTransition { .. }),
        "{error:?}"
    );
    journal.shutdown().expect("shutdown");
    let content = fs::read_to_string(&record_path).expect("record");
    assert!(content.contains("\"stage\":\"stage\""), "{content}");
    assert!(content.contains("stage/preflight"), "{content}");
    assert!(content.contains("stage/finalization"), "{content}");
}
