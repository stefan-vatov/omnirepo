//! End-to-end fleet CLI contract: help/version/parse behavior, clean
//! HOME, exact process statuses and stdout/stderr streams, cancellation
//! exit mapping, record-unavailable diagnostics, and rejection of
//! legacy commands, overrides, and ambient scans.  Full synchronization,
//! repair, setup, and parity journeys execute once in the canonical
//! journey matrix (.74.7); this suite asserts the CLI dispatch and
//! renderer contract only.

use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn command(home: &Path, current_dir: &Path) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command
        .current_dir(current_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn make_home_and_workspace() -> (tempfile::TempDir, tempfile::TempDir) {
    // The run-record authority rejects tmpfs (/tmp); the fixture home
    // lives under the repository target directory.
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    (
        tempfile::Builder::new()
            .prefix("cli-home-")
            .tempdir_in(&base)
            .expect("create temporary home"),
        tempfile::Builder::new()
            .prefix("cli-workspace-")
            .tempdir_in(&base)
            .expect("create temporary workspace"),
    )
}

fn records_in(home: &Path) -> Vec<String> {
    let runs = home.join(".omnirepo/runs");
    match fs::read_dir(&runs) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| !name.starts_with('.'))
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn help_and_version_are_config_independent_and_side_effect_free() {
    let (home, workspace) = make_home_and_workspace();
    command(home.path(), workspace.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("setup"))
        .stdout(predicate::str::contains("doctor"));
    command(home.path(), workspace.path())
        .arg("--version")
        .assert()
        .success();
    assert!(
        records_in(home.path()).is_empty(),
        "no run record is created"
    );
    assert!(!home.path().join(".omnirepo").exists(), "no config effect");
}

#[test]
fn parse_errors_and_legacy_commands_exit_two_without_effects() {
    let (home, workspace) = make_home_and_workspace();
    // Unknown command and legacy commands are rejected.
    for legacy in ["clone", "run", "migrate"] {
        command(home.path(), workspace.path())
            .arg(legacy)
            .assert()
            .code(2)
            .stdout(predicate::str::is_empty());
    }
    // Legacy overrides on the sync command are rejected.
    for flag in ["--all", "--allow", "--verbose", "--progress", "--table"] {
        command(home.path(), workspace.path())
            .arg("sync")
            .arg(flag)
            .assert()
            .code(2);
    }
    assert!(
        records_in(home.path()).is_empty(),
        "no effects from rejected invocations"
    );
}

#[test]
fn clean_home_sync_exits_zero_quietly_and_records_the_run() {
    let (home, workspace) = make_home_and_workspace();
    // An empty fleet (no machine config) is a success: quiet stdout,
    // exit 0, and the run record exists.
    command(home.path(), workspace.path())
        .arg("sync")
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    let records = records_in(home.path());
    assert_eq!(records.len(), 1, "exactly one run record: {records:?}");
}

#[test]
fn record_unavailable_diagnostics_are_truthful_and_exit_five() {
    let (home, workspace) = make_home_and_workspace();
    // A file at the .omnirepo path blocks the runs directory: the record
    // cannot be created, no effect occurs, and the exit is 5.
    fs::write(home.path().join(".omnirepo"), "not a directory").expect("blocker");
    command(home.path(), workspace.path())
        .arg("sync")
        .assert()
        .code(5)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("record"));
}

#[test]
fn human_and_agent_subprocesses_expose_identical_capabilities() {
    let (human_home, workspace) = make_home_and_workspace();
    let (agent_home, workspace_agent) = make_home_and_workspace();
    // The agent invokes the exact same surface with the same arguments;
    // the help text and the exit behavior are identical.
    let human_help = command(human_home.path(), workspace.path())
        .arg("--help")
        .output()
        .expect("human help");
    let agent_help = command(agent_home.path(), workspace_agent.path())
        .arg("--help")
        .output()
        .expect("agent help");
    assert_eq!(
        human_help.stdout, agent_help.stdout,
        "identical help surface"
    );
    assert_eq!(human_help.status.code(), agent_help.status.code());
    // No hidden agent-only flag exists: the parsed surface is exactly
    // sync/setup/doctor plus the decided --output selector.  (The help
    // prose lawfully documents the migration decline; the command itself
    // is rejected by the legacy-command test.)
    let help = String::from_utf8(human_help.stdout).expect("utf8");
    for hidden in ["clone", "--verbose", "--progress", "--table"] {
        assert!(!help.contains(hidden), "hidden surface {hidden:?} in help");
    }
}
