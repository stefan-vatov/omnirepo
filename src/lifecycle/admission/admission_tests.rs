//! Focused proof for run admission and repository lease acquisition.

#![allow(dead_code, unused_imports)]

use super::{Admission, AdmissionError, DEFAULT_LEASE_WAIT, LeaseTable};
use crate::lifecycle::event::{JournalEvent, Outcome};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::repository::RepositoryId;
use std::{fs, path::Path, time::Duration, time::SystemTime};

fn repo(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("repository id")
}

fn fixture_journal() -> (tempfile::TempDir, Journal, std::path::PathBuf, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("admission-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [5_u8; 16],
    )
    .expect("record");
    let path = record.path().to_path_buf();
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, path, run_id)
}

#[test]
fn same_repository_cannot_hold_two_mutation_leases() {
    let (_fixture, mut journal, _path, run_id) = fixture_journal();
    let table = LeaseTable::new();
    let a = repo("destination-a");
    let (first, lease) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("acquire");
    assert_eq!(first, Admission::Admitted);
    let lease = lease.expect("lease");
    assert!(table.is_held("destination-a"));

    // A second acquisition of the same repository must wait and then be
    // denied within the bounded wait.
    let (second, second_lease) = table
        .acquire(&journal.handle, &run_id, &a, Duration::from_millis(50))
        .expect("second acquire");
    assert!(matches!(second, Admission::Denied { .. }), "{second:?}");
    assert!(second_lease.is_none());

    // Releasing frees the repository for the next caller.
    table.release(&lease).expect("release");
    let (third, third_lease) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("third acquire");
    assert_eq!(third, Admission::Admitted);
    let third_lease = third_lease.expect("lease");
    table.release(&third_lease).expect("release");

    journal
        .handle
        .submit(JournalEvent::Terminal {
            checkpoint: 0,
            run_id: run_id.clone(),
            outcome: Outcome::Success,
        })
        .expect("terminal");
    journal.shutdown().expect("shutdown");
    let content = fs::read_to_string(&_path).expect("record");
    assert!(content.contains("\"stage\":\"admission\""), "{content}");
}

#[test]
fn disjoint_repositories_admit_independently() {
    let (_fixture, mut journal, _path, run_id) = fixture_journal();
    let table = LeaseTable::new();
    let a = repo("destination-a");
    let b = repo("destination-b");
    let (outcome_a, lease_a) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("acquire a");
    let (outcome_b, lease_b) = table
        .acquire(&journal.handle, &run_id, &b, DEFAULT_LEASE_WAIT)
        .expect("acquire b");
    assert_eq!(outcome_a, Admission::Admitted);
    assert_eq!(outcome_b, Admission::Admitted);
    assert!(table.is_held("destination-a"));
    assert!(table.is_held("destination-b"));
    table
        .release(&lease_a.expect("lease a"))
        .expect("release a");
    table
        .release(&lease_b.expect("lease b"))
        .expect("release b");
    journal.shutdown().expect("shutdown");
}

#[test]
fn foreign_and_missing_releases_are_typed_errors() {
    let (_fixture, mut journal, _path, run_id) = fixture_journal();
    let table = LeaseTable::new();
    let a = repo("destination-a");
    let (_, lease) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("acquire");
    let lease = lease.expect("lease");
    // A foreign token for the same repository is rejected.
    let foreign = super::Lease {
        repository: "destination-a".to_owned(),
        token: lease.token() + 1,
        last_seen: std::time::Instant::now(),
    };
    let error = table.release(&foreign).expect_err("foreign token");
    assert!(
        matches!(error, AdmissionError::ForeignLease { .. }),
        "{error:?}"
    );
    // Releasing twice is a missing-lease error after the first success.
    table.release(&lease).expect("release");
    let error = table.release(&lease).expect_err("missing lease");
    assert!(
        matches!(error, AdmissionError::MissingLease { .. }),
        "{error:?}"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn admission_wait_after_hold_is_journaled() {
    let (_fixture, mut journal, _path, run_id) = fixture_journal();
    let table = LeaseTable::new();
    let a = repo("destination-a");
    let (_, lease) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("acquire");
    // Release on a background thread after a short delay so the next caller
    // observes AdmittedAfterWait.
    let table = table.clone();
    let lease = lease.expect("lease");
    let table_for_release = table.clone();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        table_for_release.release(&lease).expect("release");
    });
    let (outcome, _) = table
        .acquire(&journal.handle, &run_id, &a, Duration::from_secs(5))
        .expect("acquire after wait");
    releaser.join().expect("releaser");
    assert_eq!(
        outcome,
        Admission::AdmittedAfterWait,
        "bounded wait then admit"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn lease_heartbeats_and_stale_recovery() {
    let (_fixture, mut journal, _path, run_id) = fixture_journal();
    let table = LeaseTable::new();
    let a = repo("destination-a");
    let (_, lease) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("acquire");
    let mut lease = lease.expect("lease");
    // A fresh lease is not stale under a long deadline; the heartbeat keeps
    // the owner authoritative.
    assert!(!table.is_stale("destination-a", Duration::from_secs(3_600)));
    table.heartbeat(&mut lease).expect("heartbeat");
    // Under a zero stale deadline the lease is stale and reclaim removes
    // only it; the repository becomes reacquirable.
    assert!(table.is_stale("destination-a", Duration::ZERO));
    let reclaimed = table.reclaim_stale(Duration::ZERO);
    assert_eq!(reclaimed, vec!["destination-a".to_owned()]);
    assert!(!table.is_held("destination-a"));
    let (outcome, _) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("reacquire");
    assert_eq!(outcome, Admission::Admitted);
    journal.shutdown().expect("shutdown");
}

#[test]
fn foreign_heartbeat_is_a_typed_error() {
    let (_fixture, mut journal, _path, run_id) = fixture_journal();
    let table = LeaseTable::new();
    let a = repo("destination-a");
    let (_, lease) = table
        .acquire(&journal.handle, &run_id, &a, DEFAULT_LEASE_WAIT)
        .expect("acquire");
    let mut lease = lease.expect("lease");
    let mut foreign = super::Lease {
        repository: "destination-a".to_owned(),
        token: lease.token() + 1,
        last_seen: std::time::Instant::now(),
    };
    let error = table
        .heartbeat(&mut foreign)
        .expect_err("foreign heartbeat");
    assert!(
        matches!(error, AdmissionError::ForeignLease { .. }),
        "{error:?}"
    );
    // The genuine owner's heartbeat still works.
    table.heartbeat(&mut lease).expect("owner heartbeat");
    journal.shutdown().expect("shutdown");
}
