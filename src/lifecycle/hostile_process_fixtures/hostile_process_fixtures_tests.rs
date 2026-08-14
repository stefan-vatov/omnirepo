//! Focused proof for the hostile verifier, Git transport, and agent
//! process fixtures.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::hostile_process_fixtures::{
    Capability, ProcessFixtureKind, hostile_process_corpus, materialize_process,
};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

#[test]
fn every_process_fixture_documents_its_attack_and_fail_boundary() {
    let corpus = hostile_process_corpus();
    assert!(!corpus.is_empty());
    for fixture in &corpus {
        assert!(!fixture.name.is_empty());
        assert!(
            !fixture.attack.is_empty(),
            "{} documents its attack",
            fixture.name
        );
        assert!(
            !fixture.expected_fail_boundary.is_empty(),
            "{} documents its expected fail boundary",
            fixture.name
        );
        assert!(!fixture.script.is_empty(), "{} has a script", fixture.name);
    }
}

#[test]
fn verifier_git_and_agent_classes_are_all_covered() {
    let corpus = hostile_process_corpus();
    let kinds = corpus
        .iter()
        .map(|fixture| fixture.kind)
        .collect::<BTreeSet<_>>();
    for required in [
        ProcessFixtureKind::VerifierCrash,
        ProcessFixtureKind::VerifierHang,
        ProcessFixtureKind::VerifierGarbage,
        ProcessFixtureKind::GitEscape,
        ProcessFixtureKind::GitHang,
        ProcessFixtureKind::GitWrongRef,
        ProcessFixtureKind::AgentEscape,
        ProcessFixtureKind::AgentFlood,
        ProcessFixtureKind::AgentCrash,
        ProcessFixtureKind::AgentHang,
    ] {
        assert!(
            kinds.contains(&required),
            "missing hostile process class {required:?}"
        );
    }
}

#[test]
fn materialized_scripts_stay_under_the_harness_root_and_are_executable() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("hostile-process-")
        .tempdir_in(&base)
        .expect("fixture");
    for entry in hostile_process_corpus() {
        let path = materialize_process(&entry, fixture.path()).expect("materialize");
        assert!(
            path.starts_with(fixture.path()),
            "escaped the harness root: {}",
            path.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("meta").permissions().mode();
            assert!(mode & 0o111 != 0, "{} is executable", path.display());
        }
    }
}

#[test]
fn the_agent_crash_script_exits_with_its_typed_code() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("hostile-agent-crash-")
        .tempdir_in(&base)
        .expect("fixture");
    let corpus = hostile_process_corpus();
    let crash = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::AgentCrash)
        .expect("agent crash fixture");
    let path = materialize_process(crash, fixture.path()).expect("materialize");
    let status = Command::new(&path)
        .current_dir(fixture.path())
        .status()
        .expect("run");
    assert_eq!(status.code(), Some(7), "the agent crash exits 7");
}

#[test]
fn the_verifier_garbage_script_emits_bounded_garbage_to_stdout() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("hostile-verifier-garbage-")
        .tempdir_in(&base)
        .expect("fixture");
    let corpus = hostile_process_corpus();
    let garbage = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::VerifierGarbage)
        .expect("garbage fixture");
    let path = materialize_process(garbage, fixture.path()).expect("materialize");
    let output = Command::new(&path)
        .current_dir(fixture.path())
        .output()
        .expect("run");
    assert!(
        !output.stdout.is_empty(),
        "the garbage verifier floods stdout"
    );
    assert!(output.stdout.len() <= 4096, "the garbage is bounded");
}
