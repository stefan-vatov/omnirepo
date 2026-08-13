//! Focused proof for deterministic agent-adapter resolution.

#![allow(dead_code, unused_imports)]

use super::{AdapterOutcome, executable_name, resolve_adapters};
use crate::configuration::AgentKind;
use std::{fs, path::Path, path::PathBuf};

fn fixture_bin(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("adapters-home-")
        .tempdir_in(&base)
        .expect("fixture");
    let bin = fixture.path().join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let executable = bin.join(name);
    fs::write(&executable, "#!/bin/sh\nexit 0\n").expect("executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    (fixture, bin, executable)
}

#[test]
fn empty_priority_is_none_configured() {
    let outcome = resolve_adaptors(&[], &[]);
    assert_eq!(outcome, AdapterOutcome::NoneConfigured);
}

fn resolve_adaptors(priority: &[AgentKind], configured: &[PathBuf]) -> AdapterOutcome {
    resolve_adapters(priority, configured).expect("resolution")
}

#[test]
fn order_is_stable_and_identities_resolve() {
    let (_fixture, bin, codex) = fixture_bin("codex");
    let (_fixture2, _bin2, claude) = fixture_bin("claude");
    let configured = vec![
        codex.parent().expect("parent").to_path_buf(),
        claude.parent().expect("parent").to_path_buf(),
    ];
    let outcome = resolve_adaptors(&[AgentKind::Codex, AgentKind::ClaudeCode], &configured);
    let AdapterOutcome::Resolved(adapters) = outcome else {
        panic!("expected resolved: {outcome:?}");
    };
    assert_eq!(adapters.len(), 2);
    assert_eq!(adapters[0].kind, AgentKind::Codex);
    assert_eq!(adapters[1].kind, AgentKind::ClaudeCode);
    assert!(adapters[0].executable.ends_with("codex"));
    assert!(!adapters[0].identity.is_empty(), "identity must resolve");
    let _ = bin;
}

#[test]
fn absent_executables_are_excluded_and_exhaustion_fails() {
    let (_fixture, bin, _codex) = fixture_bin("codex");
    // Only codex exists; claude is absent.  PATH is pinned to the fixture so
    // the ambient machine cannot satisfy the missing entry.
    let previous_path = std::env::var_os("PATH");
    unsafe { std::env::set_var("PATH", &bin) };
    let configured = vec![bin.clone()];
    let outcome = resolve_adaptors(&[AgentKind::Codex, AgentKind::ClaudeCode], &configured);
    let AdapterOutcome::Resolved(adapters) = outcome else {
        panic!("expected partial resolution: {outcome:?}");
    };
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].kind, AgentKind::Codex);

    // A non-empty priority with every executable absent is exhausted (PATH
    // is still pinned to the fixture bin).
    let outcome = resolve_adaptors(&[AgentKind::ClaudeCode, AgentKind::Pi], &[]);
    match outcome {
        AdapterOutcome::Exhausted { missing } => assert_eq!(missing.len(), 2),
        other => panic!("expected exhausted, got {other:?}"),
    }
    match &previous_path {
        Some(value) => unsafe { std::env::set_var("PATH", value) },
        None => unsafe { std::env::remove_var("PATH") },
    }
}

#[test]
fn executable_replacement_changes_the_identity() {
    let (_fixture, _bin, executable) = fixture_bin("codex");
    let configured = vec![executable.parent().expect("parent").to_path_buf()];
    let first = resolve_adaptors(&[AgentKind::Codex], &configured);
    let AdapterOutcome::Resolved(adapters) = first else {
        panic!("expected resolved");
    };
    let first_identity = adapters[0].identity.clone();
    // Replace the executable: the identity must change.
    fs::write(&executable, "#!/bin/sh\nexit 0\n# replaced\n").expect("replace");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let second = resolve_adaptors(&[AgentKind::Codex], &configured);
    let AdapterOutcome::Resolved(adapters) = second else {
        panic!("expected resolved");
    };
    assert_ne!(
        first_identity, adapters[0].identity,
        "replacement must be detected"
    );
}

#[test]
fn executable_names_map_stably() {
    assert_eq!(executable_name(AgentKind::Codex), "codex");
    assert_eq!(executable_name(AgentKind::ClaudeCode), "claude");
    assert_eq!(executable_name(AgentKind::Pi), "pi");
}
