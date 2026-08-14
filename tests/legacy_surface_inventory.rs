//! The legacy-surface inventory contract: the binary surface and the
//! docs must not claim removed surfaces as available, and the inventory
//! stays structured.

use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn command() -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command.env("HOME", Path::new(env!("CARGO_MANIFEST_DIR")).join("target"));
    command
}

#[test]
fn the_binary_help_never_claims_removed_surfaces_as_commands() {
    command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("setup"))
        .stdout(predicate::str::contains("validate"))
        // The removed surfaces never appear as invocable commands.
        .stdout(predicate::str::contains("clone").not())
        .stdout(predicate::str::contains("run").not());
}

#[test]
fn the_breaks_inventory_is_structured_and_complete() {
    let inventory =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/breaks-inventory.md"))
            .expect("inventory exists");
    // Every break row names a removed surface, a replacement, and a
    // guidance pointer.
    for break_row in [
        "multi-repo-run",
        "tag-clone",
        "ad-hoc-sync",
        "legacy-config",
        "orchestrator",
        "output",
    ] {
        assert!(
            inventory.contains(&format!("docs/breaking-guidance.md#{break_row}")),
            "missing guidance pointer for {break_row}"
        );
    }
    // The inventory lists the still-present misleading texts.
    assert!(inventory.contains("not available in this build"));
    assert!(inventory.contains("Legacy general orchestration surfaces"));
}

#[test]
fn the_removed_surfaces_are_rejected_by_the_binary() {
    // Legacy commands are rejected with an argument error.
    command().arg("clone").assert().code(2);
    command().arg("run").assert().code(2);
}
