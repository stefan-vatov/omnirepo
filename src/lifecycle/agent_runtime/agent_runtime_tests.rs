//! Focused proof for agent evidence capture and termination.

use super::{AgentRuntimeError, MAX_EVIDENCE_BYTES, run_agent};
use crate::lifecycle::agent_confinement::confine;
use crate::lifecycle::agent_framing::sanitize_output;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("agent-runtime-")
        .tempdir_in(&base)
        .expect("fixture");
    let workdir = fixture.path().join("destination");
    fs::create_dir_all(&workdir).expect("workdir");
    let evidence = fixture.path().join("evidence");
    (fixture, workdir, evidence)
}

fn confinement(workdir: &Path) -> crate::lifecycle::agent_confinement::AgentConfinement {
    let root = crate::platform::AuthorityRoot::<
        crate::platform::AgentWorkingDirectoryRoot,
        crate::platform::ReadOnly,
    >::open(workdir)
    .expect("root");
    confine(&root, &[], &[]).expect("confine")
}

fn shell_argv(script: &str) -> Vec<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    vec![shell, "-c".to_owned(), script.to_owned()]
}

#[test]
fn agent_output_is_captured_sanitized_and_written_as_evidence() {
    let (_fixture, workdir, evidence) = fixture();
    let captured = run_agent(
        &shell_argv("printf 'secret=ab12cd34ef\nok\n'"),
        &confinement(&workdir),
        &evidence,
        Duration::from_secs(10),
    )
    .expect("run");
    assert!(
        fs::metadata(&captured.evidence_path)
            .expect("evidence")
            .len()
            > 0
    );
    assert!(captured.sanitized.contains("ok"), "{}", captured.sanitized);
    assert_eq!(captured.sanitized, sanitize_output(&captured.sanitized));
    assert!(
        captured.sanitized.contains("secret"),
        "{}",
        captured.sanitized
    );
}

#[test]
fn crashing_agent_terminates_typed() {
    let (_fixture, workdir, evidence) = fixture();
    let error = run_agent(
        &shell_argv("exit 3"),
        &confinement(&workdir),
        &evidence,
        Duration::from_secs(10),
    )
    .expect_err("crash");
    assert!(
        matches!(error, AgentRuntimeError::Crashed { code: Some(3) }),
        "{error}"
    );
}

#[test]
fn hanging_agent_is_terminated_at_the_timeout() {
    let (_fixture, workdir, evidence) = fixture();
    let started = std::time::Instant::now();
    let error = run_agent(
        &shell_argv("trap '' TERM; while :; do :; done"),
        &confinement(&workdir),
        &evidence,
        Duration::from_millis(300),
    )
    .expect_err("timeout");
    assert!(
        matches!(error, AgentRuntimeError::Timeout { .. }),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "terminated promptly"
    );
}

#[test]
fn overflowing_output_is_bounded() {
    let (_fixture, workdir, evidence) = fixture();
    let captured = run_agent(
        &shell_argv("PATH=/usr/bin:/bin; head -c 1048576 /dev/zero | tr '\\0' 'x'"),
        &confinement(&workdir),
        &evidence,
        Duration::from_secs(10),
    )
    .expect("run");
    assert!(
        captured.sanitized.len() <= MAX_EVIDENCE_BYTES,
        "{} > {}",
        captured.sanitized.len(),
        MAX_EVIDENCE_BYTES
    );
}
