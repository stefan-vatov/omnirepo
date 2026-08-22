//! Typed authority root integration contract (surface scan).
//!
//! Every effectful Git consumer enters only through its typed owning root:
//! the commit journal, the push, the reconciliation, and the publication
//! freeze all require `AuthorityRoot<GitWorkingDirectoryRoot, ReadOnly>`.
//! This scan pins those signatures; the runtime proofs (invalid roots fail
//! before mutation, valid peers finish) live in the crate's
//! authority_integration tests.

use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

/// The lifecycle Git-entry signatures must require the typed root: this is
/// the seam that keeps repository-domain internals reachable only through
/// an authority-validated path.
#[test]
fn lifecycle_git_entries_require_the_typed_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, signature) in [
        (
            "src/lifecycle/commit_journal.rs",
            "pub fn create_commit_journaled(",
        ),
        (
            "src/lifecycle/commit_journal.rs",
            "pub fn reconcile_commit(",
        ),
        ("src/lifecycle/remote_push.rs", "pub fn push_recorded_oid("),
        ("src/lifecycle/push_reconcile.rs", "pub fn reconcile_push("),
        (
            "src/lifecycle/remote_target.rs",
            "pub fn freeze_remote_target(",
        ),
    ] {
        let source = fs::read_to_string(root.join(file)).expect("read entry file");
        let block = source
            .split(signature)
            .nth(1)
            .expect("entry signature must exist");
        assert!(
            block.contains("AuthorityRoot<")
                && block.contains("GitWorkingDirectoryRoot")
                && block.contains("ReadOnly"),
            "{file} entry {signature} must require the typed Git root"
        );
    }
}

/// The agent confinement entry requires the typed agent root.
#[test]
fn agent_confinement_requires_the_typed_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("src/lifecycle/agent_confinement.rs"))
        .expect("read entry file");
    let block = source
        .split("pub fn confine(")
        .nth(1)
        .expect("confine entry");
    assert!(
        block.contains("AuthorityRoot<") && block.contains("AgentWorkingDirectoryRoot"),
        "confine must require the typed agent root"
    );
}

/// The source readers require the typed read-only source root.
#[test]
fn source_readers_require_the_typed_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let file = "src/lifecycle/source_catalog.rs";
    let signature = "pub fn read_source_declarations(";
    let source = fs::read_to_string(root.join(file)).expect("read entry file");
    let block = source
        .split(signature)
        .nth(1)
        .expect("entry signature must exist");
    assert!(
        block.contains("AuthorityRoot<") && block.contains("SourceSnapshotRoot"),
        "{file} entry {signature} must require the typed source root"
    );
}

/// The run-record consumer requires the typed mutation root.
#[test]
fn run_record_requires_the_typed_mutation_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(root.join("src/lifecycle/run_record.rs")).expect("read entry file");
    assert!(
        source.contains("open_mutation_root") && source.contains("RunRecordRoot"),
        "run-record effects must use the typed RunRecordRoot mutation root"
    );
}

/// The private binary still carries no runner surface.
#[test]
fn private_binary_keeps_no_runner_surface() {
    let mut command = cargo_bin_cmd!("omnirepo");
    command.arg("--help");
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("doctor"));
}
