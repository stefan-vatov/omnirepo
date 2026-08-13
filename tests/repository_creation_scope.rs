use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

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

fn snapshot_tree(root: &Path) -> BTreeSet<PathBuf> {
    let mut snapshot = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries {
            let entry = entry.expect("read isolated test entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("snapshot entry is below its root")
                .to_path_buf();
            snapshot.insert(relative);
            if path.is_dir() {
                pending.push(path);
            }
        }
    }

    snapshot
}

#[test]
fn removed_repository_creation_commands_and_aliases_have_no_side_effects() {
    let home = TempDir::new().expect("create isolated home");
    let workspace = TempDir::new().expect("create isolated workspace");
    let template_seed = workspace.path().join("legacy-template/seed.txt");
    fs::create_dir_all(template_seed.parent().expect("template parent"))
        .expect("create template fixture");
    fs::write(&template_seed, "must remain untouched\n").expect("write template fixture");

    let legacy_commands = ["new", "new-repo", "new_repo", "create", "init", "scaffold"];
    for command_name in legacy_commands {
        let destination = workspace.path().join(format!("created-{command_name}"));
        let before_home = snapshot_tree(home.path());
        let before_workspace = snapshot_tree(workspace.path());

        command(home.path(), workspace.path())
            .args([
                command_name,
                "--name",
                "first-project",
                "--destination",
                destination.to_str().expect("destination is valid UTF-8"),
            ])
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("unrecognized subcommand")
                    .or(predicate::str::contains("unexpected argument")),
            );

        assert_eq!(
            before_home,
            snapshot_tree(home.path()),
            "{command_name} must not write global configuration"
        );
        assert_eq!(
            before_workspace,
            snapshot_tree(workspace.path()),
            "{command_name} must not create a destination or copy a template"
        );
        assert!(
            !destination.exists(),
            "{command_name} must not create a destination directory"
        );
        assert!(
            !destination.join("first-project").exists(),
            "{command_name} must not create a repository directory"
        );
        assert!(
            !destination.join(".git").exists(),
            "{command_name} must not initialize Git"
        );
        assert_eq!(
            fs::read_to_string(&template_seed).expect("read template fixture"),
            "must remain untouched\n",
            "{command_name} must not alter template material"
        );
    }
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

#[test]
fn source_and_package_surfaces_expose_no_repository_creation_api() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    for removed_file in [
        "src/new/mod.rs",
        "src/new/project_creation.rs",
        "src/new/project_creation_tests.rs",
    ] {
        assert!(
            !root.join(removed_file).is_file(),
            "removed repository-creation file still exists: {removed_file}"
        );
    }

    let mut source_files = Vec::new();
    collect_files(&root.join("src"), &mut source_files);
    source_files.extend([
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
        root.join("README.md"),
    ]);

    let forbidden_surface = [
        "pub mod new",
        "Commands::New",
        "new_repo",
        "project_creation",
        "ProjectCreation",
        "copy_templates",
        "template_and_dest",
        "filename_from_url",
        "init_repo",
        "git init",
        "git_init",
        "Repository created",
        "new-repo",
        "create-repo",
        "alias = \"new\"",
        "visible_alias",
    ];

    for path in source_files {
        let contents = fs::read_to_string(&path).expect("read source or package file");
        for marker in forbidden_surface {
            assert!(
                !contents.contains(marker),
                "repository-creation marker {marker:?} remains in {}",
                path.display()
            );
        }
    }

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let cargo_lock = fs::read_to_string(root.join("Cargo.lock")).expect("read Cargo.lock");
    for removed_dependency in ["duct", "indicatif", "rayon", "reqwest", "prettytable-rs"] {
        assert!(
            !cargo_toml.lines().any(|line| {
                line.trim_start()
                    .starts_with(&format!("{removed_dependency} ="))
            }),
            "repository-creation dependency {removed_dependency} remains in Cargo.toml"
        );
        assert!(
            !cargo_lock
                .lines()
                .any(|line| { line.trim() == format!("name = \"{removed_dependency}\"") }),
            "repository-creation dependency {removed_dependency} remains in Cargo.lock"
        );
    }
}

/// Setup and sync must not create repositories, .git directories, or
/// arbitrary directories today: the creation boundary holds before the
/// setup/onboarding workstream lands.
#[test]
fn setup_and_sync_create_no_repository_or_git_artifacts() {
    let home = TempDir::new().expect("create isolated home");
    let workspace = TempDir::new().expect("create isolated workspace");
    for (label, arguments) in [("setup", vec!["setup", "--apply"]), ("sync", vec!["sync"])] {
        let before_home = snapshot_tree(home.path());
        let before_workspace = snapshot_tree(workspace.path());
        command(home.path(), workspace.path())
            .args(arguments)
            .assert()
            // The application service is not in this build: both commands
            // exit with the typed unavailable codes and create nothing.
            .code(predicate::function(|code: &i32| *code == 2 || *code == 5));
        if label == "setup" {
            assert_eq!(
                before_home,
                snapshot_tree(home.path()),
                "{label} must not write global configuration"
            );
        }
        assert_eq!(
            before_workspace,
            snapshot_tree(workspace.path()),
            "{label} must not create directories or repositories"
        );
        assert!(
            !workspace.path().join(".git").exists(),
            "{label} must not initialize Git"
        );
    }
}
