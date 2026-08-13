//! Black-box contract for the owner-approved constitutional CLI surface (.27).
//! Help/version/parse must be config-independent and side-effect-free; the
//! tree is exactly sync/setup/validate; legacy and migrate surfaces are
//! rejected; every command fails closed until the lifecycle slices land.

use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::TempDir;

fn command(home: &Path, current_dir: &Path) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command
        .current_dir(current_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn make_home_and_workspace() -> (TempDir, TempDir) {
    (
        TempDir::new().expect("create temporary home"),
        TempDir::new().expect("create temporary workspace"),
    )
}

fn assert_home_still_empty(home: &Path) {
    let mut entries = fs::read_dir(home).expect("read temporary home");
    assert!(
        entries.next().is_none(),
        "help/version/parse must create no files or directories"
    );
}

#[test]
fn help_declares_only_constitutional_commands() {
    let (home, workspace) = make_home_and_workspace();
    let output = command(home.path(), workspace.path())
        .arg("--help")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("help is UTF-8");
    for expected in ["sync", "setup", "validate"] {
        assert!(
            stdout.contains(expected),
            "help must declare {expected:?}:\n{stdout}"
        );
    }
    for forbidden in ["run", "new", "clone", "migrate"] {
        assert!(
            !stdout.lines().any(|line| line.trim() == forbidden),
            "help must not declare legacy or migrate command {forbidden:?}:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("--output"),
        "help must declare the machine-readable output flag:\n{stdout}"
    );
    assert_home_still_empty(home.path());
}

#[test]
fn help_and_version_are_config_independent_and_side_effect_free() {
    let (home, workspace) = make_home_and_workspace();
    command(home.path(), workspace.path())
        .arg("--help")
        .assert()
        .success();
    assert_home_still_empty(home.path());

    let version = command(home.path(), workspace.path())
        .arg("--version")
        .assert()
        .success();
    assert_eq!(
        String::from_utf8(version.get_output().stdout.clone()).expect("version is UTF-8"),
        format!("omnirepo {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_home_still_empty(home.path());

    command(home.path(), workspace.path())
        .arg("-V")
        .assert()
        .success();
    assert_home_still_empty(home.path());
}

#[test]
fn legacy_and_migrate_commands_are_rejected_with_exit_two() {
    let (home, workspace) = make_home_and_workspace();
    for forbidden in ["run", "new", "clone", "migrate"] {
        command(home.path(), workspace.path())
            .arg(forbidden)
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("unrecognized subcommand")
                    .or(predicate::str::contains("unexpected argument")),
            );
    }
    assert_home_still_empty(home.path());
}

#[test]
fn constitutional_commands_fail_closed_until_the_lifecycle_lands() {
    let (home, workspace) = make_home_and_workspace();
    for name in ["sync", "setup", "validate"] {
        let output = command(home.path(), workspace.path())
            .arg(name)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("not available in this build"));
        let stderr =
            String::from_utf8(output.get_output().stderr.clone()).expect("stderr is UTF-8");
        assert!(
            stderr.contains(&format!("{name} is not available")),
            "stub must name the command: {stderr}"
        );
    }
    assert_home_still_empty(home.path());
}

#[test]
fn output_json_flag_is_global_and_invalid_values_exit_two() {
    let (home, workspace) = make_home_and_workspace();
    // Global flag before the subcommand.
    command(home.path(), workspace.path())
        .args(["--output", "json", "sync"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not available in this build"));
    // Global flag after the subcommand.
    command(home.path(), workspace.path())
        .args(["sync", "--output", "json"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not available in this build"));
    // Invalid value is an invocation error.
    command(home.path(), workspace.path())
        .args(["--output", "bogus", "sync"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"));
    assert_home_still_empty(home.path());
}

#[test]
fn setup_accepts_its_plan_apply_flag() {
    let (home, workspace) = make_home_and_workspace();
    command(home.path(), workspace.path())
        .args(["setup", "--apply"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not available in this build"));
    command(home.path(), workspace.path())
        .args(["setup", "--apply", "unexpected"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unexpected argument"));
    assert_home_still_empty(home.path());
}
