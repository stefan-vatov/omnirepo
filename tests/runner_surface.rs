use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

const FORBIDDEN_RUNNER_MARKERS: &[&str] = &[
    "run_command",
    "Commands::Run",
    "pub mod run",
    "omnirepo_lib::run",
    "omnirepo run",
    "Run a command",
    "--command",
    "sh -c",
    "Command::new(\"sh\")",
    "[\"sh\", \"-c\"]",
    "duct::cmd",
    "shell-string",
];

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).expect("read public surface directory");
    for entry in entries {
        let path = entry.expect("read public surface entry").path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

#[test]
fn private_binary_surface_contains_no_runner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let binary = fs::read_to_string(root.join("src/main.rs")).expect("read binary surface");

    for marker in [
        "pub mod run",
        "run_command",
        "Commands::Run",
        "omnirepo_lib::run",
    ] {
        assert!(
            !binary.contains(marker),
            "forbidden runner marker {marker:?} remains in src/main.rs"
        );
    }
}

#[test]
fn help_rejects_run_with_the_decided_parse_status() {
    cargo_bin_cmd!("omnirepo")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("run").not())
        .stdout(predicate::str::contains("runner").not())
        .stdout(predicate::str::contains("--command").not());

    for alias in [
        "run",
        "runner",
        "run-command",
        "run_command",
        "exec",
        "execute",
        "shell",
    ] {
        cargo_bin_cmd!("omnirepo")
            .args([alias, "--command", "true"])
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("unrecognized subcommand")
                    .or(predicate::str::contains("unexpected argument")),
            );
    }
}

#[test]
fn production_and_package_surfaces_contain_no_runner_or_shell_alias() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_files(&root.join("src"), &mut files);
    files.extend([
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
        root.join("README.md"),
        root.join("CHANGELOG.md"),
    ]);

    for path in files {
        let contents = fs::read_to_string(&path).expect("read public surface file");
        for marker in FORBIDDEN_RUNNER_MARKERS {
            assert!(
                !contents.contains(marker),
                "forbidden runner marker {marker:?} remains in {}",
                path.display()
            );
        }
    }
}
