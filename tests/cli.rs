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

#[test]
fn help_is_available_without_a_global_config() {
    let (home, workspace) = make_home_and_workspace();

    command(home.path(), workspace.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: omnirepo"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("setup"))
        .stdout(predicate::str::contains("validate"))
        .stdout(predicate::str::contains("--output"));

    command(home.path(), workspace.path())
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: omnirepo"));
}

#[test]
fn version_is_available_with_long_and_short_flags() {
    let (home, workspace) = make_home_and_workspace();
    let version = env!("CARGO_PKG_VERSION");

    command(home.path(), workspace.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(version));

    command(home.path(), workspace.path())
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains(version));
}

#[test]
fn clap_rejects_unknown_commands_and_invalid_arguments_with_exit_two() {
    let (home, workspace) = make_home_and_workspace();

    command(home.path(), workspace.path())
        .arg("unknown-command")
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("unrecognized subcommand")
                .or(predicate::str::contains("unexpected argument")),
        );

    for arguments in [
        vec!["new", "--name", "first-project"],
        vec!["run", "--command", "true"],
        vec!["clone"],
        vec!["sync", "--file", "target.txt"],
    ] {
        command(home.path(), workspace.path())
            .args(arguments)
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("unrecognized subcommand")
                    .or(predicate::str::contains("unexpected argument")),
            );
    }
}

#[test]
fn removed_new_command_is_rejected_without_creating_a_repository() {
    let (home, workspace) = make_home_and_workspace();

    let destination = workspace.path().join("projects");
    fs::create_dir_all(&destination).expect("create project destination");

    command(home.path(), workspace.path())
        .args([
            "new",
            "--name",
            "first-project",
            "--destination",
            destination.to_str().expect("destination is valid UTF-8"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));

    assert!(!destination.join("first-project").exists());
}
