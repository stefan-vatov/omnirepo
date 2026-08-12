use super::run_command;
use crate::config::parser::RepoConfig;
use std::{fs, path::Path};
use tempfile::tempdir;

fn write_repo_config(destination: &Path, dirs: &[&str]) {
    let config = RepoConfig::new(dirs.iter().map(|dir| (*dir).to_owned()).collect());
    let contents = yaml_serde::to_string(&config).expect("serialize test repo config");
    fs::write(destination.join(".omni.yaml"), contents).expect("write test repo config");
}

fn create_repositories(destination: &Path, dirs: &[&str]) {
    for dir in dirs {
        fs::create_dir_all(destination.join(dir)).expect("create test repository");
    }
}

#[test]
fn empty_repository_list_does_not_run_a_command() {
    let temp = tempdir().expect("create temporary directory");
    write_repo_config(temp.path(), &[]);

    let result = run_command(
        "this command must not run".to_owned(),
        Some(temp.path().display().to_string()),
    );

    assert!(result.is_ok());
}

#[test]
fn missing_config_error_identifies_the_config_path() {
    let temp = tempdir().expect("create temporary directory");
    let config_path = temp.path().join(".omni.yaml");

    let error = run_command("true".to_owned(), Some(temp.path().display().to_string()))
        .expect_err("missing config should fail");

    let message = error.to_string();
    assert!(message.contains("Could not open local repo config file"));
    assert!(message.contains(&config_path.display().to_string()));
}

#[test]
fn malformed_config_error_identifies_the_config_path() {
    let temp = tempdir().expect("create temporary directory");
    let config_path = temp.path().join(".omni.yaml");
    fs::write(&config_path, "dirs: [").expect("write malformed config");

    let error = run_command("true".to_owned(), Some(temp.path().display().to_string()))
        .expect_err("malformed config should fail");

    let message = error.to_string();
    assert!(message.contains("Error parsing repo config YAML file"));
    assert!(message.contains(&config_path.display().to_string()));
}

#[cfg(unix)]
#[test]
fn all_repositories_run_when_paths_contain_spaces() {
    let temp = tempdir().expect("create temporary directory");
    let destination = temp.path().join("destination with spaces");
    let dirs = ["repo one", "repo two"];
    fs::create_dir_all(&destination).expect("create destination");
    create_repositories(&destination, &dirs);
    write_repo_config(&destination, &dirs);

    let result = run_command(
        "touch .omni-ran".to_owned(),
        Some(destination.display().to_string()),
    );

    assert!(result.is_ok());
    for dir in dirs {
        assert!(destination.join(dir).join(".omni-ran").exists());
    }
}

#[cfg(unix)]
#[test]
fn missing_repository_does_not_stop_other_repositories() {
    let temp = tempdir().expect("create temporary directory");
    let destination = temp.path().join("destination");
    let existing = ["repo one", "repo two"];
    let dirs = ["repo one", "missing repo", "repo two"];
    fs::create_dir_all(&destination).expect("create destination");
    create_repositories(&destination, &existing);
    write_repo_config(&destination, &dirs);

    let error = run_command(
        "touch .omni-ran".to_owned(),
        Some(destination.display().to_string()),
    )
    .expect_err("missing repository should fail the aggregate run");

    assert!(error.to_string().contains("missing repo"));
    for dir in existing {
        assert!(destination.join(dir).join(".omni-ran").exists());
    }
}

#[cfg(unix)]
#[test]
fn nonzero_repository_command_does_not_stop_other_repositories() {
    let temp = tempdir().expect("create temporary directory");
    let destination = temp.path().join("destination");
    let dirs = ["successful repo", "failing repo"];
    fs::create_dir_all(&destination).expect("create destination");
    create_repositories(&destination, &dirs);
    fs::write(destination.join("failing repo").join(".fail"), "fail")
        .expect("write failure marker");
    write_repo_config(&destination, &dirs);

    let command = "touch .omni-ran; if test -f .fail; then exit 7; fi";
    let error = run_command(command.to_owned(), Some(destination.display().to_string()))
        .expect_err("nonzero repository command should fail the aggregate run");

    assert!(error.to_string().contains("failing repo"));
    for dir in dirs {
        assert!(destination.join(dir).join(".omni-ran").exists());
    }
}

#[cfg(unix)]
#[test]
fn invalid_repository_paths_do_not_escape_destination_or_stop_valid_repositories() {
    let temp = tempdir().expect("create temporary directory");
    let destination = temp.path().join("destination");
    let valid = destination.join("valid repo");
    let outside = temp.path().join("outside");
    let absolute = outside.display().to_string();
    let dirs: [&str; 3] = ["valid repo", "../outside", absolute.as_str()];
    fs::create_dir_all(&valid).expect("create valid repository");
    fs::create_dir_all(&destination).expect("create destination");
    write_repo_config(&destination, &dirs);

    let error = run_command(
        "touch .omni-ran".to_owned(),
        Some(destination.display().to_string()),
    )
    .expect_err("invalid repository paths should fail the aggregate run");

    let message = error.to_string();
    assert!(message.contains("2 repository command(s) failed"));
    assert!(message.contains("parent-directory traversal"));
    assert!(message.contains("an absolute path"));
    assert!(valid.join(".omni-ran").exists());
    assert!(!outside.join(".omni-ran").exists());
}
