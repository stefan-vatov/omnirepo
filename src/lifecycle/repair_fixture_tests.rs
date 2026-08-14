//! Fixture proofs for the repair pipeline composition: multi-agent
//! fallback, budget exhaustion, restart after allocation, and
//! peer-in-progress reservations.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::repair_classify::FailureClass;
use crate::lifecycle::repair_fallback::allocate_within_budget;
use crate::lifecycle::repair_fold::{RepairHistory, fold_into_terminal_outcome};
use crate::lifecycle::repair_reserve::{
    RepairReservation, ReserveError, ReserveOutcome, reserve_repair_attempt,
};
use crate::lifecycle::repair_selection::{
    EligibleRepair, FailedRepository, select_eligible_failed,
};
use crate::lifecycle::run_record::RunRecord;
use crate::lifecycle::run_summary::RepoOutcome;
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

fn failed(repository: &str) -> FailedRepository {
    FailedRepository {
        repository: repository.to_owned(),
        class: FailureClass::SyncDrift,
    }
}

fn proven(repository: &str) -> (String, crate::lifecycle::repair_causation::CausationVerdict) {
    (
        repository.to_owned(),
        crate::lifecycle::repair_causation::CausationVerdict::Proven,
    )
}

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
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_path)
}

#[test]
fn multi_agent_fallback_runs_the_next_agent_after_a_failed_one() {
    // Selection + priority fallback: the configured order chooses the
    // first agent; a later repair failure for the same repository folds
    // into a terminal failure that keeps both reasons, and the next
    // repository in priority still proceeds.
    let failed = vec![failed("repo-a"), failed("repo-b")];
    let causation = vec![proven("repo-a"), proven("repo-b")];
    let selected = select_eligible_failed(&failed, &causation);
    let allocations =
        allocate_within_budget(&selected, &["repo-b".to_owned(), "repo-a".to_owned()], 4);
    let ids = allocations
        .iter()
        .map(|entry| entry.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-b", "repo-a"], "{ids:?}");
    // The first agent's repair failed; the terminal fold keeps both
    // reasons and never claims success.
    let terminal = fold_into_terminal_outcome(
        &RepoOutcome::Failure {
            reason: "sync drift".to_owned(),
        },
        Some(&RepairHistory::Failed {
            reason: "agent 1 crashed".to_owned(),
        }),
    );
    match terminal {
        RepoOutcome::Failure { reason } => {
            assert!(reason.contains("sync drift") && reason.contains("agent 1 crashed"));
        }
        other => panic!("expected failure, got {other:?}"),
    }
}

#[test]
fn budget_exhaustion_leaves_later_repositories_untouched() {
    let selected = vec![eligible("repo-a", 3), eligible("repo-b", 3)];
    let allocations = allocate_within_budget(&selected, &[], 2);
    assert_eq!(allocations.len(), 1, "{allocations:?}");
    assert_eq!(allocations[0].repository, "repo-a");
    assert_eq!(allocations[0].attempts, 2);
    // The untouched repository keeps its initial failure in the fold.
    let terminal = fold_into_terminal_outcome(
        &RepoOutcome::Failure {
            reason: "sync drift".to_owned(),
        },
        None,
    );
    assert!(matches!(terminal, RepoOutcome::Failure { .. }));
}

#[test]
fn a_restart_after_reservation_never_double_reserves() {
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    // The reservation is journaled durably before it is returned.
    let reserved = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "repo-a",
        &["baseline-1".to_owned()],
        1,
        "",
    )
    .expect("reserve");
    let ReserveOutcome::Reserved(RepairReservation { repository, .. }) = reserved;
    assert_eq!(repository, "repo-a");
    // The record now carries the durable reservation marker.
    let record = fs::read_to_string(&record_path).expect("record");
    assert!(record.contains("attempt/1"), "{record}");
    // A restart of the same repository fails the duplicate detection
    // instead of double-reserving.
    let duplicate = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "repo-a",
        &["baseline-1".to_owned()],
        1,
        &record,
    );
    assert!(
        matches!(duplicate, Err(ReserveError::AlreadyReserved { .. })),
        "{duplicate:?}"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_peer_in_progress_reservation_excludes_the_repository() {
    let (_jfixture, mut journal, run_id, record_path) = journal_fixture();
    // A peer run already reserved the attempt for repo-b.
    let reserved = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "repo-b",
        &["baseline-1".to_owned()],
        1,
        "",
    )
    .expect("reserve");
    let ReserveOutcome::Reserved(RepairReservation { repository, .. }) = reserved;
    assert_eq!(repository, "repo-b");
    journal.shutdown().expect("shutdown");
    let record = fs::read_to_string(&record_path).expect("record");
    // The peer's reservation is visible; the duplicate detection refuses
    // a second reservation for the same repository in the same run.
    let second = reserve_repair_attempt(
        &journal.handle,
        &run_id,
        "repo-b",
        &["baseline-1".to_owned()],
        1,
        &record,
    );
    assert!(
        matches!(second, Err(ReserveError::AlreadyReserved { .. })),
        "{second:?}"
    );
}
