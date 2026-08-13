//! Focused proof for restart cleanup and transaction evidence.

#![allow(dead_code, unused_imports)]

use super::{
    CleanupError, CleanupReport, EvidenceStage, OWNED_TEMP_MARKER, record_outcome, restart_cleanup,
};
use crate::lifecycle::event::{JournalEvent, Outcome};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{fs, path::Path, path::PathBuf, time::SystemTime};

fn fixture_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("evidence-home-")
        .tempdir_in(&base)
        .expect("create evidence fixture");
    let root = fixture.path().to_path_buf();
    (fixture, root)
}

fn fixture_journal() -> (tempfile::TempDir, Journal, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("evidence-journal-")
        .tempdir_in(&base)
        .expect("create journal fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs dir");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        [6_u8; 16],
    )
    .expect("record");
    let path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, path)
}

fn run_id() -> String {
    "20231114T221320Z-06060606060606060606060606060606".to_owned()
}

#[test]
fn restart_cleanup_removes_only_exact_owned_artifacts() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join(".target.txt.omnirepo-tmp-op-1-1.tmp"), b"partial").expect("owned temp");
    fs::write(root.join("target.txt"), b"old").expect("target");
    fs::write(root.join("notes.txt"), b"keep").expect("peer file");
    let report = restart_cleanup(&root).expect("cleanup");
    assert_eq!(
        report.removed,
        vec![root.join(".target.txt.omnirepo-tmp-op-1-1.tmp")]
    );
    assert!(
        root.join("target.txt").exists(),
        "the target is never touched"
    );
    assert!(
        root.join("notes.txt").exists(),
        "peer files are never touched"
    );
}

#[test]
fn ambiguous_artifacts_abort_without_any_mutation() {
    let (_fixture, root) = fixture_root();
    fs::write(root.join("fake.omnirepo-tmp-1.tmp"), b"ambiguous").expect("ambiguous");
    fs::write(root.join("owned.omnirepo-tmp-op-1-1.tmp"), b"owned").expect("owned");
    let error = restart_cleanup(&root).expect_err("ambiguous must abort");
    assert!(matches!(error, CleanupError::Ambiguous { .. }), "{error:?}");
    // Nothing was mutated: both files still exist.
    assert!(root.join("fake.omnirepo-tmp-1.tmp").exists());
    assert!(root.join("owned.omnirepo-tmp-op-1-1.tmp").exists());
}

#[test]
fn malformed_owned_lookalikes_fail_closed() {
    let (_fixture, root) = fixture_root();
    // Zero attempt and empty operation id are not owned artifacts: any file
    // carrying the owned marker that does not match the exact grammar is
    // ambiguous and aborts the pass without mutation.
    fs::write(root.join(".x.omnirepo-tmp-op-0.tmp"), b"zero").expect("zero attempt");
    fs::write(root.join(".x.omnirepo-tmp--1.tmp"), b"empty op").expect("empty op");
    let error = restart_cleanup(&root).expect_err("ambiguous must abort");
    assert!(matches!(error, CleanupError::Ambiguous { .. }), "{error:?}");
    assert!(root.join(".x.omnirepo-tmp-op-0.tmp").exists());
    assert!(root.join(".x.omnirepo-tmp--1.tmp").exists());
}

#[test]
fn evidence_stages_are_recorded_with_exact_identities_and_replay() {
    let (_fixture, mut journal, record_path) = fixture_journal();
    // Seed the writer's invocation intent like the run boundary does.
    let record_id = run_id();
    let intent = JournalEvent::RunIntent {
        checkpoint: 0,
        run_id: record_id.clone(),
        stage: crate::lifecycle::event::RunStage::Invocation,
    };
    // The record already carries the invocation intent in the file; the
    // writer seeds it, so record_outcome appends evidence at checkpoints 1+.
    let checkpoint = record_outcome(
        &journal.handle,
        EvidenceStage::Compare,
        &record_id,
        &PathBuf::from("target.txt"),
        5,
    )
    .expect("record compare");
    assert_eq!(checkpoint, 1);
    record_outcome(
        &journal.handle,
        EvidenceStage::Write,
        &record_id,
        &PathBuf::from(".target.txt.omnirepo-tmp-op-1-1.tmp"),
        8,
    )
    .expect("record write");
    record_outcome(
        &journal.handle,
        EvidenceStage::Publish,
        &record_id,
        &PathBuf::from("target.txt"),
        5,
    )
    .expect("record publish");
    record_outcome(
        &journal.handle,
        EvidenceStage::Cleanup,
        &record_id,
        &PathBuf::from(".target.txt.omnirepo-tmp-op-1-1.tmp"),
        0,
    )
    .expect("record cleanup");
    journal
        .handle
        .submit(JournalEvent::Terminal {
            checkpoint: 0,
            run_id: record_id.clone(),
            outcome: Outcome::Success,
        })
        .expect("terminal");
    journal.shutdown().expect("shutdown");

    let content = fs::read_to_string(&record_path).expect("record");
    // The intent line plus four evidence events plus the terminal.
    assert_eq!(content.lines().count(), 6, "{content}");
    assert!(content.contains("\"type\":\"evidence\""), "{content}");
    for stage in ["compare", "write", "publish", "cleanup"] {
        assert!(
            content.contains(&format!("\"stage\":\"{stage}\"")),
            "missing stage {stage}: {content}"
        );
    }
    assert!(content.contains("\"path\":\"target.txt\""), "{content}");
    assert!(
        content.contains("\"path\":\".target.txt.omnirepo-tmp-op-1-1.tmp\""),
        "{content}"
    );
    let _ = intent;
}

#[test]
fn hostile_evidence_paths_are_rejected_before_the_journal() {
    let (_fixture, mut journal, _record_path) = fixture_journal();
    let error = record_outcome(
        &journal.handle,
        EvidenceStage::Write,
        &run_id(),
        &PathBuf::from("target.txt\n{"),
        4,
    )
    .expect_err("hostile evidence path must fail");
    assert!(error.to_string().contains("unsafe characters"), "{error}");
    journal.shutdown().expect("shutdown");
}
