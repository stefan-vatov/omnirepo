//! Stale-lease recovery and cancellation-over-admission fixtures.

#![allow(dead_code, unused_imports)]

use super::{Admission, LeaseTable};
use crate::lifecycle::cancellation::cancel_run;
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::repository::RepositoryId;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("repository id")
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("admission-concurrency-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [6_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id)
}

#[test]
fn stale_reclaim_recovers_only_dead_leases() {
    let (_fixture, journal, run_id) = journal_fixture();
    let table = LeaseTable::new();
    let (admission, live) = table
        .acquire(
            &journal.handle,
            &run_id,
            &repository_id("dest-a"),
            Duration::ZERO,
        )
        .expect("acquire a");
    assert!(matches!(admission, Admission::Admitted));
    let mut live = live.expect("lease a");
    // A second repository takes its lease; then the live lease is
    // heartbeated so it stays fresh.
    let (_admission_b, _lease_b) = table
        .acquire(
            &journal.handle,
            &run_id,
            &repository_id("dest-b"),
            Duration::ZERO,
        )
        .expect("acquire b");
    std::thread::sleep(Duration::from_millis(20));
    table.heartbeat(&mut live).expect("heartbeat");
    let reclaimed = table.reclaim_stale(Duration::from_millis(10));
    // dest-b aged (acquired 20ms ago, no heartbeat) and is reclaimed;
    // dest-a was heartbeated and stays live.
    assert!(reclaimed.contains(&"dest-b".to_owned()), "{reclaimed:?}");
    assert!(!reclaimed.contains(&"dest-a".to_owned()), "{reclaimed:?}");
    assert!(table.is_held("dest-a"));
    assert!(!table.is_held("dest-b"));
    table.release(&live).expect("release");
}

#[test]
fn cancelled_run_rejects_new_leases() {
    let (_fixture, journal, run_id) = journal_fixture();
    let table = LeaseTable::new();
    let (admission, lease) = table
        .acquire(
            &journal.handle,
            &run_id,
            &repository_id("dest-a"),
            Duration::ZERO,
        )
        .expect("acquire");
    assert!(matches!(admission, Admission::Admitted));
    // The run is cancelled while the lease is in flight; the lease table
    // stays consistent (the holder can still release), but the journal is
    // now terminal.
    cancel_run(&journal.handle, &run_id, &["dest-a".to_owned()]).expect("cancel");
    let mut lease = lease.expect("lease");
    table
        .heartbeat(&mut lease)
        .expect("heartbeat stays table-consistent");
    table.release(&lease).expect("release");
    // A new admission attempt on the cancelled run is refused by the
    // journal (the run is terminal), and no lease is left behind.
    let error = table
        .acquire(
            &journal.handle,
            &run_id,
            &repository_id("dest-b"),
            Duration::ZERO,
        )
        .expect_err("admission after cancel");
    assert!(format!("{error}").contains("terminal"), "{error}");
    assert!(
        !table.is_held("dest-b"),
        "rejected admission must not hold a lease"
    );
}
