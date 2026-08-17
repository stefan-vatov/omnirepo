//! Focused proof for frozen verification followed by the authorized-delta
//! revalidation gate.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::command_spec::CommandSpec;
use crate::lifecycle::verification_gate::{GateInputs, GateVerdict};
use crate::lifecycle::verify_and_gate::{VerificationRun, VerificationVerdict, verify_and_gate};
use crate::managed_content::CompareOutcome;
use crate::platform::RelativePath;
use std::{fs, path::Path, time::Duration};

fn spec(argv: &[&str]) -> CommandSpec {
    spec_at(argv, 0)
}

fn spec_at(argv: &[&str], position: usize) -> CommandSpec {
    CommandSpec {
        repository: "dest-a".to_owned(),
        plan_identity: "plan-1".to_owned(),
        position,
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: RelativePath::root(),
        env: vec![],
        timeout: Duration::from_secs(10),
        stdin: None,
        capture_output: true,
        shell: None,
    }
}

fn gate() -> GateInputs {
    GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-1".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-1".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    }
}

fn fixture() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    tempfile::Builder::new()
        .prefix("verify-gate-")
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn passing_checks_and_a_pass_gate_allow_git_delivery() {
    let fixture = fixture();
    let run = verify_and_gate(
        fixture.path(),
        &[spec(&["/usr/bin/true"])],
        &gate(),
        CompareOutcome::Unchanged,
        true,
        false,
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(run.checks.len(), 1);
    assert_eq!(run.verdict, VerificationVerdict::Ready);
    assert_eq!(run.gate, GateVerdict::Pass);
}

#[test]
fn a_failed_check_prevents_git_delivery() {
    let fixture = fixture();
    let run = verify_and_gate(
        fixture.path(),
        &[spec(&["/usr/bin/false"])],
        &gate(),
        CompareOutcome::Unchanged,
        true,
        false,
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(run.verdict, VerificationVerdict::FailedCheck);
}

#[test]
fn gate_drift_or_identity_change_prevents_git_delivery() {
    let fixture = fixture();
    let run = verify_and_gate(
        fixture.path(),
        &[spec(&["/usr/bin/true"])],
        &gate(),
        CompareOutcome::Unchanged,
        false,
        false,
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(run.verdict, VerificationVerdict::GateRejected);
    assert_eq!(run.gate, GateVerdict::ForbiddenDelta);
    // Concurrent modification also rejects.
    let run = verify_and_gate(
        fixture.path(),
        &[spec(&["/usr/bin/true"])],
        &gate(),
        CompareOutcome::Unchanged,
        true,
        true,
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(run.verdict, VerificationVerdict::GateRejected);
}

#[test]
fn checks_run_in_declared_order_with_bounded_results() {
    let fixture = fixture();
    let run = verify_and_gate(
        fixture.path(),
        &[
            spec_at(&["/usr/bin/true"], 0),
            spec_at(&["/usr/bin/true"], 1),
        ],
        &gate(),
        CompareOutcome::Unchanged,
        true,
        false,
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(run.checks.len(), 2);
    assert_eq!(run.checks[0].position, 0);
    assert_eq!(run.checks[1].position, 1);
    let _: VerificationRun = run;
}
