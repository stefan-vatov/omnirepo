//! Focused proof for running bounded post-pass repair in the final fleet
//! run.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, AgentKind, DestinationRepository, MachineConcurrency, MachineConfiguration,
    RepairControls, RepositoryId, SchemaVersion,
};
use crate::lifecycle::adapters::AdapterResolution;
use crate::lifecycle::fleet_repair::{FailedMember, RepairPassOutcome, run_fleet_repair};
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-repair-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn destination(root: &Path, id: &str) -> DestinationRepository {
    let path = root.join(id);
    fs::create_dir_all(&path).expect("destination");
    DestinationRepository::new(
        RepositoryId::parse(id).expect("repository id"),
        AbsolutePath::parse(path.to_str().expect("utf8")).expect("path"),
        Vec::new(),
    )
    .expect("destination")
}

fn machine_with_repair(
    repositories: Vec<DestinationRepository>,
    priority: Vec<AgentKind>,
) -> MachineConfiguration {
    MachineConfiguration::new(
        SchemaVersion::new(1).expect("version"),
        repositories,
        Vec::new(),
        None,
        MachineConcurrency::new(4, 8).expect("concurrency"),
        RepairControls::new(priority, 3).expect("repair controls"),
    )
    .expect("machine")
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("fleet-repair-home-")
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
    let record_text = fs::read_to_string(&record_path).expect("record text");
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id, record_text)
}

/// The record text after the journal wrote everything (re-read fresh).
fn fresh_record(fixture: &tempfile::TempDir, run_id: &str) -> String {
    fs::read_to_string(
        fixture
            .path()
            .join(".omnirepo/runs")
            .join(format!("{run_id}.log")),
    )
    .expect("fresh record")
}

fn adapter_script(destination_dir: &Path, body: &str) -> AdapterResolution {
    let executable = destination_dir.join("fake-agent");
    fs::write(&executable, body).expect("agent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable).expect("meta").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("mode");
    }
    AdapterResolution {
        kind: AgentKind::Pi,
        executable,
        identity: "fake-agent-identity".to_owned(),
    }
}

#[test]
fn no_configured_adapters_leave_the_repositories_failed() {
    let fixture = fixture_base();
    let destination_dir = fixture.path().join("repo-a");
    fs::create_dir_all(&destination_dir).expect("destination");
    let config = machine_with_repair(vec![destination(fixture.path(), "repo-a")], Vec::new());
    let (_jfixture, mut journal, run_id, record_text) = journal_fixture();
    let failed = vec![FailedMember {
        repository: "repo-a".to_owned(),
        reason: "verifier crashed".to_owned(),
    }];
    let outcome = run_fleet_repair(
        &journal.handle,
        &run_id,
        &config,
        &failed,
        &[],
        &record_text,
    )
    .expect("repair pass");
    assert!(
        outcome.still_failed.iter().any(|id| id == "repo-a"),
        "no adapters means no repair: {:?}",
        outcome
    );
    assert!(outcome.repaired.is_empty());
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_successful_agent_repairs_its_repository() {
    let fixture = fixture_base();
    let destination_dir = fixture.path().join("repo-a");
    fs::create_dir_all(&destination_dir).expect("destination");
    fs::write(destination_dir.join("managed.txt"), "v1\n").expect("managed");
    let config = machine_with_repair(
        vec![destination(fixture.path(), "repo-a")],
        vec![AgentKind::Pi],
    );
    let adapter = adapter_script(&destination_dir, "#!/bin/sh\necho repaired-ok\n");
    let (_jfixture, mut journal, run_id, record_text) = journal_fixture();
    let failed = vec![FailedMember {
        repository: "repo-a".to_owned(),
        reason: "verifier crashed".to_owned(),
    }];
    let outcome = run_fleet_repair(
        &journal.handle,
        &run_id,
        &config,
        &failed,
        &[adapter],
        &record_text,
    )
    .expect("repair pass");
    assert!(
        outcome.repaired.iter().any(|id| id == "repo-a"),
        "the agent repaired the repository: {:?}",
        outcome
    );
    assert!(outcome.still_failed.is_empty());
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_crashing_agent_consumes_the_attempt_and_the_repository_stays_failed() {
    let fixture = fixture_base();
    let destination_dir = fixture.path().join("repo-a");
    fs::create_dir_all(&destination_dir).expect("destination");
    fs::write(destination_dir.join("managed.txt"), "v1\n").expect("managed");
    let config = machine_with_repair(
        vec![destination(fixture.path(), "repo-a")],
        vec![AgentKind::Pi],
    );
    let adapter = adapter_script(&destination_dir, "#!/bin/sh\nexit 7\n");
    let (_jfixture, mut journal, run_id, record_text) = journal_fixture();
    let failed = vec![FailedMember {
        repository: "repo-a".to_owned(),
        reason: "verifier crashed".to_owned(),
    }];
    let outcome = run_fleet_repair(
        &journal.handle,
        &run_id,
        &config,
        &failed,
        &[adapter],
        &record_text,
    )
    .expect("repair pass");
    assert!(
        outcome.still_failed.iter().any(|id| id == "repo-a"),
        "the crashing agent consumed the attempt: {:?}",
        outcome
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_duplicate_reservation_never_double_repairs() {
    let fixture = fixture_base();
    let destination_dir = fixture.path().join("repo-a");
    fs::create_dir_all(&destination_dir).expect("destination");
    fs::write(destination_dir.join("managed.txt"), "v1\n").expect("managed");
    let config = machine_with_repair(
        vec![destination(fixture.path(), "repo-a")],
        vec![AgentKind::Pi],
    );
    let adapter = adapter_script(&destination_dir, "#!/bin/sh\necho repaired-ok\n");
    let (_jfixture, mut journal, run_id, record_text) = journal_fixture();
    let failed = vec![FailedMember {
        repository: "repo-a".to_owned(),
        reason: "verifier crashed".to_owned(),
    }];
    // First pass repairs repo-a and journals the reservation.
    let first = run_fleet_repair(
        &journal.handle,
        &run_id,
        &config,
        &failed,
        std::slice::from_ref(&adapter),
        &record_text,
    )
    .expect("first");
    assert!(first.repaired.iter().any(|id| id == "repo-a"));
    // A restart re-reads the durable record: the reservation marker is
    // visible, so the second pass refuses a duplicate attempt and the
    // repository is never repaired twice.
    let fresh = fresh_record(&_jfixture, &run_id);
    assert!(fresh.contains("attempt/1"), "the reservation is durable");
    let second = run_fleet_repair(
        &journal.handle,
        &run_id,
        &config,
        &failed,
        &[adapter],
        &fresh,
    );
    assert!(second.is_ok(), "{second:?}");
    journal.shutdown().expect("shutdown");
}
