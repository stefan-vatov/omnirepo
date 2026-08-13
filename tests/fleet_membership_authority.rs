use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::TempDir;

const LEGACY_SENTINEL: &str = "legacy-fleet-sentinel\n";

fn command(home: &Path, current_dir: &Path) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command
        .current_dir(current_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PWD", current_dir)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

#[test]
fn legacy_omni_and_tag_clone_cannot_select_or_create_destinations() {
    let home = TempDir::new().expect("create home fixture");
    let current_dir = TempDir::new().expect("create cwd fixture");
    let existing_destination = current_dir.path().join("existing-destination");
    let new_destination = current_dir.path().join("new-destination");
    fs::create_dir_all(&existing_destination).expect("create existing destination");

    let legacy_files = [
        home.path().join(".omni.yaml"),
        home.path().join(".omnirepo.yaml"),
        current_dir.path().join(".omni.yaml"),
        current_dir.path().join(".omnirepo.yaml"),
        existing_destination.join(".omni.yaml"),
        existing_destination.join(".omnirepo.yaml"),
    ];
    for path in &legacy_files {
        fs::write(path, LEGACY_SENTINEL).expect("write legacy authority fixture");
    }

    let output = command(home.path(), current_dir.path())
        .args([
            "clone",
            "--tags",
            "legacy",
            "--destination",
            new_destination
                .to_str()
                .expect("destination is valid UTF-8"),
        ])
        .output()
        .expect("run legacy clone invocation");

    assert_eq!(output.status.code(), Some(2));
    let mut output_text = String::from_utf8_lossy(&output.stdout).into_owned();
    output_text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        output_text.contains("unrecognized subcommand")
            || output_text.contains("unexpected argument")
    );
    assert!(!output_text.contains(LEGACY_SENTINEL.trim()));
    assert!(!new_destination.exists());
    for path in &legacy_files {
        assert_eq!(
            fs::read(path).expect("read legacy authority fixture"),
            LEGACY_SENTINEL.as_bytes()
        );
    }
    assert_eq!(
        fs::read_dir(&existing_destination)
            .expect("read existing destination")
            .count(),
        2
    );
}

#[test]
fn help_is_config_independent_and_names_canonical_machine_configuration() {
    let home = TempDir::new().expect("create home fixture");
    let current_dir = TempDir::new().expect("create cwd fixture");
    let legacy_files = [
        home.path().join(".omni.yaml"),
        home.path().join(".omnirepo.yaml"),
        current_dir.path().join(".omni.yaml"),
        current_dir.path().join(".omnirepo.yaml"),
    ];
    for path in &legacy_files {
        fs::write(path, LEGACY_SENTINEL).expect("write legacy authority fixture");
    }

    command(home.path(), current_dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("<HOME>/.omnirepo/config.yaml"))
        .stdout(predicate::str::contains("version: 1"))
        .stdout(predicate::str::contains("not migrated automatically"))
        .stdout(predicate::str::contains(LEGACY_SENTINEL.trim()).not());

    for path in &legacy_files {
        assert_eq!(
            fs::read(path).expect("read legacy authority fixture"),
            LEGACY_SENTINEL.as_bytes()
        );
    }
}
