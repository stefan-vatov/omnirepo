//! Focused proof for invoking one confined agent with a causally bounded
//! repair task.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::repair_execute::{
    RepairError, RepairOutcome, RepairRequest, execute_confined_repair,
};
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
        .prefix("repair-execute-home-")
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
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id)
}

fn agent_script(path: &Path, body: &str) -> String {
    fs::write(path, body).expect("agent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("meta").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("mode");
    }
    path.display().to_string()
}

#[test]
fn a_successful_confined_agent_yields_typed_evidence() {
    let (_fixture, root) = {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        fs::create_dir_all(&base).expect("base");
        let fixture = tempfile::Builder::new()
            .prefix("repair-execute-work-")
            .tempdir_in(&base)
            .expect("fixture");
        let root = fixture.path().join("destination");
        fs::create_dir_all(&root).expect("destination");
        (fixture, root)
    };
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let agent = agent_script(&root.join("repair-agent"), "#!/bin/sh\necho repaired-ok\n");
    let outcome = execute_confined_repair(RepairRequest {
        destination: &root,
        argv: &[agent, "task".to_owned()],
        task: "repair-managed",
        journal: &journal.handle,
        run_id: &run_id,
        repository: "dest-a",
        frozen_inputs: &["baseline-1".to_owned()],
        budget: Duration::from_secs(10),
        trusted_agent: false,
    })
    .expect("repair");
    assert!(matches!(outcome, RepairOutcome::Succeeded { .. }));
    let RepairOutcome::Succeeded { evidence } = outcome;
    assert!(evidence.contains("repaired-ok"), "{evidence}");
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_crashing_agent_fails_typed_without_claiming_success() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-execute-crash-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("destination");
    fs::create_dir_all(&root).expect("destination");
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let agent = agent_script(&root.join("repair-agent"), "#!/bin/sh\nexit 7\n");
    let error = execute_confined_repair(RepairRequest {
        destination: &root,
        argv: &[agent, "task".to_owned()],
        task: "repair-managed",
        journal: &journal.handle,
        run_id: &run_id,
        repository: "dest-a",
        frozen_inputs: &["baseline-1".to_owned()],
        budget: Duration::from_secs(10),
        trusted_agent: false,
    })
    .expect_err("crash");
    assert!(matches!(error, RepairError::AgentCrashed { .. }), "{error}");
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_hanging_agent_is_terminated_at_the_budget() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-execute-hang-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("destination");
    fs::create_dir_all(&root).expect("destination");
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let agent = agent_script(
        &root.join("repair-agent"),
        "#!/bin/sh\nwhile :; do :; done\n",
    );
    let error = execute_confined_repair(RepairRequest {
        destination: &root,
        argv: &[agent, "task".to_owned()],
        task: "repair-managed",
        journal: &journal.handle,
        run_id: &run_id,
        repository: "dest-a",
        frozen_inputs: &["baseline-1".to_owned()],
        budget: Duration::from_millis(300),
        trusted_agent: false,
    })
    .expect_err("timeout");
    assert!(
        matches!(error, RepairError::AgentTimedOut { .. }),
        "{error}"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn an_escaping_agent_path_fails_before_execution() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("repair-execute-escape-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("destination");
    fs::create_dir_all(&root).expect("destination");
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let outside = agent_script(
        &fixture.path().join("outside-agent"),
        "#!/bin/sh\necho outside\n",
    );
    let error = execute_confined_repair(RepairRequest {
        destination: &root,
        argv: &[outside, "task".to_owned()],
        task: "repair-managed",
        journal: &journal.handle,
        run_id: &run_id,
        repository: "dest-a",
        frozen_inputs: &["baseline-1".to_owned()],
        budget: Duration::from_secs(10),
        trusted_agent: false,
    })
    .expect_err("escape");
    assert!(
        matches!(error, RepairError::AgentPathEscapes { .. }),
        "{error}"
    );
    journal.shutdown().expect("shutdown");
}
