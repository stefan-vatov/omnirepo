//! Focused proof for the bounded check runner.

#![allow(dead_code, unused_imports)]

use super::{CheckOutcome, MAX_CHECK_OUTPUT_BYTES, run_check};
use crate::lifecycle::command_spec::CommandSpec;
use crate::platform::RelativePath;
use std::{fs, path::Path, time::Duration};

fn spec(argv: &[&str], timeout: Duration) -> CommandSpec {
    CommandSpec {
        repository: "dest-a".to_owned(),
        plan_identity: "plan-1".to_owned(),
        position: 0,
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: RelativePath::root(),
        env: vec![],
        timeout,
        stdin: None,
        capture_output: true,
        shell: None,
    }
}

#[test]
fn passing_and_failing_checks_reach_typed_terminal_results() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("check-runner-")
        .tempdir_in(&base)
        .expect("fixture");
    let ok = run_check(
        fixture.path(),
        &spec(&["/usr/bin/true"], Duration::from_secs(10)),
        Duration::from_secs(10),
    )
    .expect("run");
    assert_eq!(ok.outcome, CheckOutcome::Passed);
    let fail = run_check(
        fixture.path(),
        &spec(&["/usr/bin/false"], Duration::from_secs(10)),
        Duration::from_secs(10),
    )
    .expect("run");
    assert!(
        matches!(fail.outcome, CheckOutcome::Failed { code: Some(1) }),
        "{:?}",
        fail.outcome
    );
}

#[test]
fn timeout_terminates_and_no_descendant_survives() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("check-timeout-")
        .tempdir_in(&base)
        .expect("fixture");
    let marker = fixture.path().join("descendant-marker");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let script = format!("(sleep 30; touch {}) & wait", marker.display());
    let started = std::time::Instant::now();
    let result = run_check(
        fixture.path(),
        &spec(&[&shell, "-c", &script], Duration::from_secs(30)),
        Duration::from_millis(300),
    )
    .expect("run");
    assert!(matches!(result.outcome, CheckOutcome::TimedOut { .. }));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "terminated promptly"
    );
    // The descendant is reaped with the group: the marker never appears.
    std::thread::sleep(Duration::from_millis(200));
    assert!(!marker.exists(), "descendant survived the termination");
}

#[test]
fn output_is_bounded_and_sanitized() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("check-output-")
        .tempdir_in(&base)
        .expect("fixture");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let result = run_check(
        fixture.path(),
        &spec(
            &[&shell, "-c", "printf '\\033[2Jok %1024s' x"],
            Duration::from_secs(10),
        ),
        Duration::from_secs(10),
    )
    .expect("run");
    assert!(result.evidence.len() <= MAX_CHECK_OUTPUT_BYTES);
    assert!(!result.evidence.contains('\u{1b}'), "sanitized");
    assert!(result.evidence.contains("ok"), "{}", result.evidence);
}
