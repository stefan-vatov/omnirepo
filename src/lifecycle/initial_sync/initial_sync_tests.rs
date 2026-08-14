//! Focused proof for the contained multi-item synchronization pass.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::initial_sync::{
    FailurePolicy, SyncItem, SyncOutcome, SyncPassReport, execute_sync_pass,
};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn journal_fixture() -> (tempfile::TempDir, Journal, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("initial-sync-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [2_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id)
}

fn item(id: &str, target: &str, frozen: &[u8], current: &[u8]) -> SyncItem {
    SyncItem {
        plan_item_id: id.to_owned(),
        target: target.to_owned(),
        frozen_bytes: frozen.to_vec(),
        current_bytes: current.to_vec(),
        fail: None,
    }
}

fn failing_item(id: &str, target: &str, frozen: &[u8], current: &[u8]) -> SyncItem {
    SyncItem {
        plan_item_id: id.to_owned(),
        target: target.to_owned(),
        frozen_bytes: frozen.to_vec(),
        current_bytes: current.to_vec(),
        fail: Some("simulated failure".to_owned()),
    }
}

#[test]
fn every_operation_has_intent_and_result_and_unchanged_performs_no_write() {
    let (_fixture, mut journal, run_id) = journal_fixture();
    let items = vec![
        item("a", "managed.txt", b"v1\n", b"v1\n"),
        item("b", "changed.txt", b"v1\n", b"v2\n"),
    ];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        &items,
        FailurePolicy::Continue,
    )
    .expect("pass");
    assert_eq!(report.items.len(), 2);
    // Unchanged: no write, typed outcome, journaled result.
    assert_eq!(report.items[0].outcome, SyncOutcome::Unchanged);
    assert!(report.items[0].journaled);
    // Changed: typed replacement outcome, journaled result.
    assert!(matches!(report.items[1].outcome, SyncOutcome::Replacement));
    assert!(report.items[1].journaled);
    journal.shutdown().expect("shutdown");
}

#[test]
fn failure_leaves_exactly_the_residue_and_later_items_follow_policy() {
    let (_fixture, mut journal, run_id) = journal_fixture();
    let items = vec![
        failing_item("a", "managed.txt", b"v1\n", b"v2\n"),
        item("b", "later.txt", b"v1\n", b"v2\n"),
    ];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        &items,
        FailurePolicy::Continue,
    )
    .expect("pass");
    // A simulated failure on the first item leaves exactly its residue
    // (the temp candidate path) and the later item still executes.
    match &report.items[0].outcome {
        SyncOutcome::Failed { residue, .. } => {
            assert_eq!(residue.len(), 1, "exactly the .55 residue");
            assert!(
                residue[0].contains("managed.txt.omnirepo-tmp"),
                "{}",
                residue[0]
            );
        }
        other => panic!("expected failed, got {other:?}"),
    }
    assert_eq!(report.items[1].outcome, SyncOutcome::Replacement);
    journal.shutdown().expect("shutdown");

    // StopOnFailure: the later item is skipped with a typed reason.
    let (_fixture2, mut journal2, run_id2) = journal_fixture();
    let report = execute_sync_pass(
        &journal2.handle,
        &run_id2,
        "dest-a",
        &items,
        FailurePolicy::StopOnFailure,
    )
    .expect("pass");
    assert!(matches!(
        report.items[0].outcome,
        SyncOutcome::Failed { .. }
    ));
    assert!(matches!(
        report.items[1].outcome,
        SyncOutcome::Skipped { .. }
    ));
    journal2.shutdown().expect("shutdown");
}

#[test]
fn outside_scope_content_is_protected() {
    let (_fixture, mut journal, run_id) = journal_fixture();
    // An escaping target fails the pass typed before any effect.
    let items = vec![item("a", "../escape.txt", b"v1\n", b"v2\n")];
    let error = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        &items,
        FailurePolicy::Continue,
    )
    .expect_err("escaping target");
    assert!(format!("{error}").contains("outside"), "{error}");
    journal.shutdown().expect("shutdown");
}

#[test]
fn report_is_ordered_and_complete() {
    let (_fixture, mut journal, run_id) = journal_fixture();
    let items = vec![
        item("a", "a.txt", b"v1\n", b"v1\n"),
        item("b", "b.txt", b"v1\n", b"v2\n"),
        item("c", "c.txt", b"v1\n", b"v3\n"),
    ];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        &items,
        FailurePolicy::Continue,
    )
    .expect("pass");
    assert_eq!(report.items.len(), 3);
    let ids: Vec<&str> = report
        .items
        .iter()
        .map(|e| e.plan_item_id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"], "declared order preserved");
    let _: SyncPassReport = report;
    journal.shutdown().expect("shutdown");
}
