//! Focused proof for running the configured priority fallback within the
//! durable repair budget.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::repair_classify::FailureClass;
use crate::lifecycle::repair_fallback::{
    RepairAllocation, allocate_within_budget, commit_repair_allocations,
};
use crate::lifecycle::repair_selection::EligibleRepair;
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn eligible(repository: &str, attempts: u8) -> EligibleRepair {
    EligibleRepair {
        repository: repository.to_owned(),
        class: FailureClass::SyncDrift,
        attempts,
        reason: "sync drift".to_owned(),
    }
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-fallback-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

#[test]
fn priority_fallback_allocates_configured_repositories_first() {
    let eligible = vec![
        eligible("repo-a", 1),
        eligible("repo-b", 1),
        eligible("repo-c", 1),
    ];
    let allocations = allocate_within_budget(&eligible, &["repo-c".to_owned()], 3);
    let ids = allocations
        .iter()
        .map(|entry| entry.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-c", "repo-a", "repo-b"], "{ids:?}");
}

#[test]
fn the_budget_caps_the_allocations_and_counts_attempts() {
    let eligible = vec![
        eligible("repo-a", 2),
        eligible("repo-b", 2),
        eligible("repo-c", 2),
    ];
    let allocations = allocate_within_budget(&eligible, &[], 4);
    let ids = allocations
        .iter()
        .map(|entry| entry.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-a", "repo-b"], "{ids:?}");
    assert_eq!(
        allocations.iter().map(|e| e.attempts).sum::<u32>(),
        4,
        "the budget is consumed exactly"
    );
}

#[test]
fn unknown_priority_entries_do_not_disturb_the_allocations() {
    let eligible = vec![eligible("repo-a", 1), eligible("repo-b", 1)];
    let allocations =
        allocate_within_budget(&eligible, &["ghost".to_owned(), "repo-b".to_owned()], 2);
    let ids = allocations
        .iter()
        .map(|entry| entry.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-b", "repo-a"], "{ids:?}");
}

#[test]
fn the_allocation_record_is_durable_in_the_journal() {
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    let allocations = vec![
        RepairAllocation {
            repository: "repo-a".to_owned(),
            attempts: 1,
        },
        RepairAllocation {
            repository: "repo-b".to_owned(),
            attempts: 2,
        },
    ];
    commit_repair_allocations(
        &journal.handle,
        &run_id,
        "destination-a",
        &allocations,
        "repair-fallback",
    )
    .expect("committed");
    journal.shutdown().expect("shutdown");
    let lines = fs::read_to_string(&record_path).expect("record");
    assert!(lines.contains("\"type\":\"evidence\""), "{lines}");
    assert!(lines.contains("repair-fallback"), "{lines}");
}
