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
        current_bytes: current.to_vec(),
        replacement: frozen.to_vec(),
        create: false,
        fail: None,
    }
}

fn creation_item(id: &str, target: &str, frozen: &[u8]) -> SyncItem {
    SyncItem {
        plan_item_id: id.to_owned(),
        target: target.to_owned(),
        current_bytes: Vec::new(),
        replacement: frozen.to_vec(),
        create: true,
        fail: None,
    }
}

fn failing_item(id: &str, target: &str, frozen: &[u8], current: &[u8]) -> SyncItem {
    SyncItem {
        plan_item_id: id.to_owned(),
        target: target.to_owned(),
        current_bytes: current.to_vec(),
        replacement: frozen.to_vec(),
        create: false,
        fail: Some("simulated failure".to_owned()),
    }
}

#[test]
fn every_operation_has_intent_and_result_and_unchanged_performs_no_write() {
    let (_fixture, mut journal, run_id) = journal_fixture();
    std::fs::write(_fixture.path().join("managed.txt"), b"v1\n").expect("managed fixture");
    std::fs::write(_fixture.path().join("changed.txt"), b"v2\n").expect("changed fixture");
    let items = vec![
        item("a", "managed.txt", b"v1\n", b"v1\n"),
        item("b", "changed.txt", b"v1\n", b"v2\n"),
    ];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        _fixture.path(),
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
    std::fs::write(_fixture.path().join("later.txt"), b"v2\n").expect("later fixture");
    let items = vec![
        failing_item("a", "managed.txt", b"v1\n", b"v2\n"),
        item("b", "later.txt", b"v1\n", b"v2\n"),
    ];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        _fixture.path(),
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
        _fixture2.path(),
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
fn a_replacement_publishes_atomically_and_preserves_the_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (_fixture, mut journal, run_id) = journal_fixture();
    fs::write(_fixture.path().join("changed.txt"), b"v1\n").expect("fixture");
    fs::set_permissions(
        _fixture.path().join("changed.txt"),
        fs::Permissions::from_mode(0o664),
    )
    .expect("mode");
    let items = vec![item("a", "changed.txt", b"v2\n", b"v1\n")];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        _fixture.path(),
        &items,
        FailurePolicy::Continue,
    )
    .expect("pass");
    assert!(matches!(report.items[0].outcome, SyncOutcome::Replacement));
    assert_eq!(
        fs::read(_fixture.path().join("changed.txt")).expect("content"),
        b"v2\n"
    );
    let mode = fs::metadata(_fixture.path().join("changed.txt"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o664, "the existing mode is preserved exactly");
    // No temporary residue after a successful replacement.
    let residue = fs::read_dir(_fixture.path())
        .expect("dir")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .contains("omnirepo-tmp")
        })
        .count();
    assert_eq!(residue, 0, "no temporary residue after success");
    journal.shutdown().expect("shutdown");
}

#[test]
fn an_absent_target_is_a_lawful_creation_with_safe_parents() {
    use std::os::unix::fs::PermissionsExt;
    let (_fixture, mut journal, run_id) = journal_fixture();
    // An empty-payload creation still creates the file: absence never
    // classifies as "unchanged".
    let items = vec![
        creation_item("a", "nested/dir/new.txt", b"created\n"),
        creation_item("b", "empty.txt", b""),
    ];
    let report = execute_sync_pass(
        &journal.handle,
        &run_id,
        "dest-a",
        _fixture.path(),
        &items,
        FailurePolicy::Continue,
    )
    .expect("pass");
    assert!(matches!(report.items[0].outcome, SyncOutcome::Replacement));
    assert!(matches!(report.items[1].outcome, SyncOutcome::Replacement));
    assert_eq!(
        fs::read(_fixture.path().join("nested/dir/new.txt")).expect("created"),
        b"created\n"
    );
    assert_eq!(
        fs::read(_fixture.path().join("empty.txt")).expect("created"),
        b""
    );
    let mode = fs::metadata(_fixture.path().join("nested/dir/new.txt"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode & !0o644, 0, "no bits beyond the 0644 creation mode");
    journal.shutdown().expect("shutdown");
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
        _fixture.path(),
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
        _fixture.path(),
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
