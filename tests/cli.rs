use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::TempDir;

const EMPTY_GLOBAL_CONFIG: &str = "repositories: []\ntemplates: []\n";

fn command(home: &Path, current_dir: &Path) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command
        .current_dir(current_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write_config(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write test config");
}

fn valid_local_config(path: &Path, dirs: &[&str]) {
    let mut contents = String::from("dirs:\n");
    for dir in dirs {
        contents.push_str("  - ");
        contents.push_str(dir);
        contents.push('\n');
    }
    write_config(path, &contents);
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
        .stdout(predicate::str::contains("clone"));

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
fn every_subcommand_help_is_available_without_a_config() {
    let (home, workspace) = make_home_and_workspace();

    for subcommand in ["new", "clone", "run", "sync"] {
        command(home.path(), workspace.path())
            .args([subcommand, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Usage:"));
    }
}

#[test]
fn clap_rejects_unknown_commands_and_invalid_arguments_with_exit_two() {
    let (home, workspace) = make_home_and_workspace();

    command(home.path(), workspace.path())
        .arg("unknown-command")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));

    command(home.path(), workspace.path())
        .args(["new", "--name"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn explicit_config_takes_precedence_over_a_malformed_default_config() {
    let (home, workspace) = make_home_and_workspace();
    write_config(
        &home.path().join(".omnirepo.yaml"),
        "repositories: [this is malformed",
    );

    let explicit_config = workspace.path().join("explicit.yaml");
    write_config(&explicit_config, EMPTY_GLOBAL_CONFIG);
    let destination = workspace.path().join("clone-destination");
    fs::create_dir_all(&destination).expect("create clone destination");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&explicit_config),
            "clone",
            "--tags",
            "not-present",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success();

    assert!(destination.join(".omni.yaml").is_file());
}

#[test]
fn documented_config_directory_is_accepted() {
    let (home, workspace) = make_home_and_workspace();
    write_config(
        &home.path().join(".omnirepo.yaml"),
        "repositories: [this is malformed",
    );

    let config_directory = workspace.path().join("config");
    fs::create_dir_all(&config_directory).expect("create config directory");
    write_config(
        &config_directory.join(".omnirepo.yaml"),
        EMPTY_GLOBAL_CONFIG,
    );

    let destination = workspace.path().join("clone-destination");
    fs::create_dir_all(&destination).expect("create clone destination");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&config_directory),
            "clone",
            "--tags",
            "not-present",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success();

    assert!(destination.join(".omni.yaml").is_file());
}

#[test]
fn verbose_mode_initializes_logging_and_dispatches_the_command() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);
    let destination = workspace.path().join("clone-destination");
    fs::create_dir_all(&destination).expect("create clone destination");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "--verbose",
            "true",
            "clone",
            "--tags",
            "not-present",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Logger set up."));

    assert!(destination.join(".omni.yaml").is_file());
}

#[test]
fn empty_or_unmatched_clone_still_writes_local_config() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);

    let destination = workspace.path().join("clone-destination");
    fs::create_dir_all(&destination).expect("create clone destination");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "clone",
            "--tags",
            "no-match",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success();

    let local_config = destination.join(".omni.yaml");
    assert!(local_config.is_file());
    assert!(
        fs::read_to_string(local_config)
            .expect("read generated local config")
            .contains("dirs")
    );
}

#[test]
fn new_creates_a_git_repository_without_network_templates() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);

    let destination = workspace.path().join("projects");
    fs::create_dir_all(&destination).expect("create project destination");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "new",
            "--name",
            "first-project",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success();

    assert!(destination.join("first-project/.git").is_dir());
}

#[test]
fn new_rejects_a_duplicate_destination() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);

    let destination = workspace.path().join("projects");
    fs::create_dir_all(&destination).expect("create project destination");
    let args = [
        "--config",
        &path_arg(&global_config),
        "new",
        "--name",
        "duplicate-project",
        "--destination",
        &path_arg(&destination),
    ];

    command(home.path(), workspace.path())
        .args(args)
        .assert()
        .success();
    command(home.path(), workspace.path())
        .args(args)
        .assert()
        .failure();
}

#[test]
fn run_executes_in_every_local_repository() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);

    let destination = workspace.path().join("repositories");
    fs::create_dir_all(destination.join("repo-a")).expect("create first repository");
    fs::create_dir_all(destination.join("repo-b")).expect("create second repository");
    valid_local_config(&destination.join(".omni.yaml"), &["repo-a", "repo-b"]);

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "run",
            "--command",
            "printf marker > run-marker.txt",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(destination.join("repo-a/run-marker.txt")).unwrap(),
        "marker"
    );
    assert_eq!(
        fs::read_to_string(destination.join("repo-b/run-marker.txt")).unwrap(),
        "marker"
    );
}

#[test]
fn run_propagates_a_child_command_failure() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);

    let destination = workspace.path().join("repositories");
    fs::create_dir_all(destination.join("repo")).expect("create repository");
    valid_local_config(&destination.join(".omni.yaml"), &["repo"]);

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "run",
            "--command",
            "exit 9",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}

#[test]
fn sync_copies_a_local_source_to_nested_targets() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);

    let destination = workspace.path().join("repositories");
    for repo in ["repo-a", "repo-b"] {
        fs::create_dir_all(destination.join(repo).join("nested"))
            .expect("create nested repository target");
    }
    fs::create_dir_all(destination.join("source")).expect("create source directory");
    write_config(
        &destination.join("source/template.txt"),
        "synced from a local source\n",
    );
    valid_local_config(&destination.join(".omni.yaml"), &["repo-a", "repo-b"]);

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "sync",
            "--file",
            "nested/shared.txt",
            "--source-file",
            "source/template.txt",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .success();

    for repo in ["repo-a", "repo-b"] {
        assert_eq!(
            fs::read_to_string(destination.join(repo).join("nested/shared.txt")).unwrap(),
            "synced from a local source\n"
        );
    }
}

#[test]
fn missing_global_config_is_reported_as_a_failure() {
    let (home, workspace) = make_home_and_workspace();
    let destination = workspace.path().join("clone-destination");
    fs::create_dir_all(&destination).expect("create clone destination");

    command(home.path(), workspace.path())
        .args([
            "clone",
            "--tags",
            "missing",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}

#[test]
fn malformed_global_config_is_reported_as_a_failure() {
    let (home, workspace) = make_home_and_workspace();
    write_config(
        &home.path().join(".omnirepo.yaml"),
        "repositories: [this is malformed",
    );
    let destination = workspace.path().join("clone-destination");
    fs::create_dir_all(&destination).expect("create clone destination");

    command(home.path(), workspace.path())
        .args([
            "clone",
            "--tags",
            "missing",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}

#[test]
fn missing_local_config_is_reported_as_a_failure() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);
    let destination = workspace.path().join("repositories");
    fs::create_dir_all(&destination).expect("create repository destination");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "run",
            "--command",
            "true",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}

#[test]
fn malformed_local_config_is_reported_as_a_failure() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);
    let destination = workspace.path().join("repositories");
    fs::create_dir_all(&destination).expect("create repository destination");
    write_config(&destination.join(".omni.yaml"), "dirs: [this is malformed");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "run",
            "--command",
            "true",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}

#[test]
fn sync_rejects_a_missing_source() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);
    let destination = workspace.path().join("repositories");
    fs::create_dir_all(&destination).expect("create repository destination");
    valid_local_config(&destination.join(".omni.yaml"), &[]);

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "sync",
            "--file",
            "missing.txt",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "sync",
            "--file",
            "missing.txt",
            "--source-file",
            "does-not-exist.txt",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}

#[test]
fn sync_rejects_conflicting_sources() {
    let (home, workspace) = make_home_and_workspace();
    let global_config = workspace.path().join("global.yaml");
    write_config(&global_config, EMPTY_GLOBAL_CONFIG);
    let destination = workspace.path().join("repositories");
    fs::create_dir_all(&destination).expect("create repository destination");
    valid_local_config(&destination.join(".omni.yaml"), &[]);
    write_config(&destination.join("source.txt"), "source");

    command(home.path(), workspace.path())
        .args([
            "--config",
            &path_arg(&global_config),
            "sync",
            "--file",
            "target.txt",
            "--source-file",
            "source.txt",
            "--url",
            "not-a-url",
            "--destination",
            &path_arg(&destination),
        ])
        .assert()
        .failure();
}
