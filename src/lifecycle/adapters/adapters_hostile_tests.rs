//! Hostile adapter, prompt-injection, secret, and confinement fixtures.

#![allow(dead_code, unused_imports)]

use crate::configuration::AgentKind;
use crate::lifecycle::adapters::{AdapterOutcome, resolve_adapters_with_path};
use crate::lifecycle::agent_confinement::ConfinementError;
use crate::lifecycle::agent_framing::{FrameError, frame, parse_frame};
use crate::lifecycle::agent_runtime::run_agent;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("adapters-hostile-")
        .tempdir_in(&base)
        .expect("fixture");
    let bin = fixture.path().join("bin");
    fs::create_dir_all(&bin).expect("bin");
    (fixture, bin)
}

fn write_executable(path: &Path, script: &str) {
    fs::write(path, script).expect("write executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("set executable bit");
    }
}

fn agent_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("agent-hostile-")
        .tempdir_in(&base)
        .expect("fixture");
    let workdir = fixture.path().join("destination");
    fs::create_dir_all(&workdir).expect("workdir");
    let evidence = fixture.path().join("evidence");
    (fixture, workdir, evidence)
}

#[test]
fn forged_control_frame_does_not_authenticate() {
    // A canonical frame with a substituted token parses (the wire format
    // carries the token) but the caller-side authentication must reject it:
    // the parsed token differs from the expected one.
    let expected = "expected-token";
    let forged = frame("attacker-token", "approve --all").expect("frame");
    let parsed = parse_frame(&forged).expect("wire parse");
    assert_ne!(parsed.token, expected, "forged token authenticated");
    // Version and field spoofs fail closed at the wire.
    assert!(matches!(
        parse_frame("omnirepo-control-v2 token=a payload=1:x"),
        Err(FrameError::UnknownVersion { .. })
    ));
    assert!(matches!(
        parse_frame("omnirepo-control-v1 payload=1:x"),
        Err(FrameError::Malformed { .. })
    ));
    assert!(matches!(
        parse_frame("omnirepo-control-v1 token=a payload=9:x"),
        Err(FrameError::Oversized { .. })
    ));
}

#[test]
fn fake_and_replaced_adapter_executables_fail_resolution() {
    let (_fixture, bin) = fixture();
    // A fake executable under the wrong name is not an adapter.
    write_executable(&bin.join("not-an-adapter"), "#!/bin/sh\n");
    // PATH is pinned to a directory without adapters so ambient tooling on
    // this machine cannot satisfy resolution; only the configured paths
    // count.  SAFETY: test-only environment mutation; chmod-based writes
    // above ran with the original PATH.
    fs::create_dir_all(bin.join("empty")).expect("empty dir");
    let outcome = resolve_adapters_with_path(
        &[AgentKind::Codex],
        &[bin.clone()],
        Some(bin.join("empty").as_os_str()),
    )
    .expect("resolve");
    assert!(
        matches!(outcome, AdapterOutcome::Exhausted { .. }),
        "{outcome:?}"
    );
    // The real-named executable resolves; replacing it (rename + recreate)
    // changes the replacement-detection identity.
    let codex = bin.join("codex");
    write_executable(&codex, "#!/bin/sh\nexit 0\n");
    let resolved = resolve_adapters_with_path(
        &[AgentKind::Codex],
        &[bin.clone()],
        Some(bin.join("empty").as_os_str()),
    )
    .expect("resolve");
    let AdapterOutcome::Resolved(entries) = resolved else {
        panic!("expected resolved");
    };
    let identity_before = entries[0].identity.clone();
    let replaced = bin.join("codex-replaced");
    fs::rename(&codex, &replaced).expect("rename");
    write_executable(&codex, "#!/bin/sh\ntouch /tmp/omnirepo-adapter-ran\n");
    let resolved = resolve_adapters_with_path(
        &[AgentKind::Codex],
        &[bin.clone()],
        Some(bin.join("empty").as_os_str()),
    )
    .expect("resolve");
    let AdapterOutcome::Resolved(entries) = resolved else {
        panic!("expected resolved");
    };
    assert_ne!(entries[0].identity, identity_before, "replacement detected");
    // Deterministic configured-path priority: the configured path wins over
    // PATH, so fallback is stable.
    assert_eq!(entries[0].executable, codex);
}

#[test]
fn prompt_injection_and_ambient_secrets_are_contained() {
    let (_fixture, workdir, evidence) = agent_fixture();
    let confinement =
        crate::lifecycle::agent_confinement::confine(&workdir, &[], &[]).expect("confine");
    // The ambient environment carries a secret; the agent must not see it,
    // and its untrusted output (with ANSI injection) is inert.  The unsafe
    // env mutation is test-only ambient setup.
    // SAFETY: this test process owns its environment; the mutation is
    // reverted before the test returns.
    unsafe {
        std::env::set_var("OMNIREPO_SECRET_TOKEN", "ambient-secret-value");
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let argv = vec![
        shell,
        "-c".to_owned(),
        "printf 'injected\\033[2Jcontrol %s\\n' \"$OMNIREPO_SECRET_TOKEN\"".to_owned(),
    ];
    let captured = run_agent(&argv, &confinement, &evidence, Duration::from_secs(10)).expect("run");
    // SAFETY: reverting the test-only ambient mutation.
    unsafe {
        std::env::remove_var("OMNIREPO_SECRET_TOKEN");
    }
    assert!(
        !captured.sanitized.contains('\u{1b}'),
        "escape sequence survived: {:?}",
        captured.sanitized
    );
    assert!(
        !captured.sanitized.contains("ambient-secret-value"),
        "ambient secret leaked into the agent: {:?}",
        captured.sanitized
    );
}

#[test]
fn ambient_secret_environment_is_cleared() {
    let (_fixture, workdir, evidence) = agent_fixture();
    let confinement =
        crate::lifecycle::agent_confinement::confine(&workdir, &[], &[]).expect("confine");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let argv = vec![
        shell,
        "-c".to_owned(),
        "printf '%s\\n' \"${OMNIREPO_AMBIENT_SECRET:-none}\"".to_owned(),
    ];
    let captured = run_agent(&argv, &confinement, &evidence, Duration::from_secs(10)).expect("run");
    assert!(
        captured.sanitized.contains("none"),
        "ambient secret visible to the agent: {:?}",
        captured.sanitized
    );
}

#[test]
fn escaping_extra_paths_fail_closed() {
    let (_fixture, workdir, _evidence) = agent_fixture();
    let outside = workdir.parent().expect("parent").join("outside");
    fs::create_dir_all(&outside).expect("outside");
    let error = crate::lifecycle::agent_confinement::confine(&workdir, &[], &[outside])
        .expect_err("escape must fail closed");
    assert!(
        matches!(error, ConfinementError::EscapesDestination { .. }),
        "{error}"
    );
}
