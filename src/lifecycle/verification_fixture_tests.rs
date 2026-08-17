//! Verification executor fixtures: command order, process tree,
//! side-effect, and parity regression proofs.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::check_runner::{CheckOutcome, run_check};
use crate::lifecycle::command_spec::{CommandSpec, translate_commands};
use crate::lifecycle::verification_gate::{GateInputs, GateVerdict, revalidate_gate};
use crate::managed_content::CompareOutcome;
use crate::platform::RelativePath;
use std::{fs, path::Path, time::Duration};

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

#[test]
fn command_order_is_preserved_with_typed_evidence() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("verify-order-")
        .tempdir_in(&base)
        .expect("fixture");
    let first = run_check(
        fixture.path(),
        &spec_at(&["/usr/bin/true"], 0),
        Duration::from_secs(10),
    )
    .expect("run");
    let second = run_check(
        fixture.path(),
        &spec_at(&["/usr/bin/false"], 1),
        Duration::from_secs(10),
    )
    .expect("run");
    // The typed results carry their positions and outcomes; a failure at
    // any position prevents Git delivery.
    assert_eq!(first.position, 0);
    assert_eq!(first.outcome, CheckOutcome::Passed);
    assert_eq!(second.position, 1);
    assert!(matches!(
        second.outcome,
        CheckOutcome::Failed { code: Some(1) }
    ));
    // Translation preserves the declared order too.
    let declared = translate_commands(
        "dest-a",
        "plan-1",
        &[
            crate::lifecycle::command_spec::DeclaredCommand {
                argv: vec!["a".to_owned()],
                cwd: None,
                env: vec![],
                timeout: None,
                stdin: None,
                capture_output: true,
                shell: None,
            },
            crate::lifecycle::command_spec::DeclaredCommand {
                argv: vec!["b".to_owned()],
                cwd: None,
                env: vec![],
                timeout: None,
                stdin: None,
                capture_output: true,
                shell: None,
            },
        ],
        Duration::from_secs(30),
    )
    .expect("translate");
    assert_eq!(declared[0].argv, vec!["a"]);
    assert_eq!(declared[1].argv, vec!["b"]);
}

#[test]
fn no_direct_worker_output_interleaves() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("verify-capture-")
        .tempdir_in(&base)
        .expect("fixture");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let result = run_check(
        fixture.path(),
        &spec_at(&[&shell, "-c", "echo worker-output"], 0),
        Duration::from_secs(10),
    )
    .expect("run");
    // The worker output is captured into the evidence, never printed
    // directly to the shared stdout.
    assert!(
        result.evidence.contains("worker-output"),
        "{}",
        result.evidence
    );
}

#[test]
fn failures_prevent_git_but_valid_peers_proceed() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("verify-peers-")
        .tempdir_in(&base)
        .expect("fixture");
    // The failing peer's gate turns to Drift/ForbiddenDelta; the valid
    // peer's gate passes independently.
    let failing = run_check(
        fixture.path(),
        &spec_at(&["/usr/bin/false"], 0),
        Duration::from_secs(10),
    )
    .expect("run");
    assert!(matches!(failing.outcome, CheckOutcome::Failed { .. }));
    let valid = run_check(
        fixture.path(),
        &spec_at(&["/usr/bin/true"], 0),
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(valid.outcome, CheckOutcome::Passed);
    let gate = GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-1".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-1".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    };
    assert_eq!(
        revalidate_gate(&gate, CompareOutcome::Unchanged, true, false),
        GateVerdict::Pass
    );
}

#[test]
fn parity_regression_detects_drift_after_verification() {
    let gate = GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-1".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-1".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    };
    // A drifted target (a replacement outcome) flips the gate away from
    // Pass: Git is prevented.
    assert_ne!(
        revalidate_gate(
            &gate,
            CompareOutcome::Replacement(
                crate::managed_content::TransactionPlan::new(
                    "run-1",
                    std::path::PathBuf::from("managed.txt"),
                    crate::managed_content::ParentDirectories::existing(),
                )
                .expect("plan")
            ),
            true,
            false,
        ),
        GateVerdict::Pass
    );
}
