use std::{fs, path::Path};

use duct::cmd;
use tempfile::{TempDir, tempdir};

use super::*;
use crate::config::{
    manager::GlobalConfigManager,
    parser::{Config, RepoConfig, Repository},
};

fn repository(name: &str, url: &Path, tags: &[&str], destination: &str) -> Repository {
    Repository {
        name: name.to_owned(),
        url: url.to_string_lossy().into_owned(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        dest: destination.to_owned(),
    }
}

fn config(repositories: Vec<Repository>) -> GlobalConfigManager {
    GlobalConfigManager::new(Config {
        repositories,
        templates: Vec::new(),
    })
}

fn init_repository(parent: &TempDir, name: &str, marker: &str) -> TempDir {
    let repository = tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(parent.path())
        .expect("create local repository directory");

    cmd!("git", "init", "--quiet")
        .dir(repository.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .run()
        .expect("initialize local repository");
    cmd!("git", "config", "user.email", "tests@example.invalid")
        .dir(repository.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .run()
        .expect("configure local repository email");
    cmd!("git", "config", "user.name", "omnirepo tests")
        .dir(repository.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .run()
        .expect("configure local repository name");
    fs::write(repository.path().join("marker.txt"), marker).expect("write local repository marker");
    cmd!("git", "add", "marker.txt")
        .dir(repository.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .run()
        .expect("stage local repository marker");
    cmd!("git", "commit", "--quiet", "--no-gpg-sign", "-m", "initial")
        .dir(repository.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .run()
        .expect("commit local repository marker");

    repository
}

fn read_repo_config(root: &Path) -> RepoConfig {
    let yaml = fs::read_to_string(root.join(".omni.yaml")).expect("read generated config");
    yaml_serde::from_str(&yaml).expect("parse generated config")
}

#[test]
fn no_matching_tags_writes_an_empty_config_without_cloning() {
    let workspace = tempdir().expect("create temporary workspace");
    let root = workspace.path().join("clone root with spaces");
    fs::create_dir(&root).expect("create clone root");
    let source = init_repository(&workspace, "unused-source", "unused");
    let cfg = config(vec![repository(
        "unused",
        source.path(),
        &["other"],
        "unused destination",
    )]);

    clone_repo(
        cfg,
        &["missing".to_owned()],
        Some(root.to_string_lossy().into_owned()),
    )
    .expect("zero matches should succeed");

    assert_eq!(read_repo_config(&root).dirs, Vec::<String>::new());
    assert!(!root.join("unused destination").exists());
}

#[test]
fn clones_each_ordered_url_destination_pair_once() {
    let workspace = tempdir().expect("create temporary workspace");
    let root = workspace.path().join("clone root with spaces");
    fs::create_dir(&root).expect("create clone root");
    let source_a = init_repository(&workspace, "source-a", "a");
    let source_b = init_repository(&workspace, "source-b", "b");
    let destination_a = "configured destination A";
    let destination_a_alt = "configured destination A alt";
    let destination_b = "configured destination B";
    let cfg = config(vec![
        repository("A", source_a.path(), &["one", "two"], destination_a),
        repository("A duplicate", source_a.path(), &["two"], destination_a),
        repository(
            "A alternate destination",
            source_a.path(),
            &["two"],
            destination_a_alt,
        ),
        repository("B", source_b.path(), &["two"], destination_b),
    ]);
    let tags = ["two".to_owned(), "one".to_owned(), "two".to_owned()];

    clone_repo(cfg, &tags, Some(root.to_string_lossy().into_owned()))
        .expect("local repositories should clone");

    let generated = read_repo_config(&root);
    assert_eq!(
        generated.dirs,
        vec![
            destination_a.to_owned(),
            destination_a_alt.to_owned(),
            destination_b.to_owned()
        ]
    );
    assert_eq!(
        fs::read_to_string(root.join(destination_a).join("marker.txt"))
            .expect("read clone A marker"),
        "a"
    );
    assert_eq!(
        fs::read_to_string(root.join(destination_a_alt).join("marker.txt"))
            .expect("read alternate clone marker"),
        "a"
    );
    assert_eq!(
        fs::read_to_string(root.join(destination_b).join("marker.txt"))
            .expect("read clone B marker"),
        "b"
    );
}

#[test]
fn reports_all_failures_and_does_not_write_a_config() {
    let workspace = tempdir().expect("create temporary workspace");
    let root = workspace.path().join("clone root");
    fs::create_dir(&root).expect("create clone root");
    let source = init_repository(&workspace, "source", "valid");
    let missing_a = workspace.path().join("missing repository A");
    let missing_b = workspace.path().join("missing repository B");
    let cfg = config(vec![
        repository("valid", source.path(), &["all"], "valid clone"),
        repository("missing-a", &missing_a, &["all"], "missing clone A"),
        repository("missing-b", &missing_b, &["all"], "missing clone B"),
    ]);

    let error = clone_repo(
        cfg,
        &["all".to_owned()],
        Some(root.to_string_lossy().into_owned()),
    )
    .expect_err("a failed clone should fail the operation");
    let error = error.to_string();

    assert!(error.contains("missing repository A"));
    assert!(error.contains("missing repository B"));
    assert!(root.join("valid clone").join("marker.txt").exists());
    assert!(!root.join(".omni.yaml").exists());
}

#[test]
fn rejects_absolute_and_parent_traversal_destinations_before_cloning() {
    for invalid_destination in ["/absolute destination", "../parent destination"] {
        let workspace = tempdir().expect("create temporary workspace");
        let root = workspace.path().join("clone root");
        fs::create_dir(&root).expect("create clone root");
        let source = init_repository(&workspace, "source", "should not clone");
        let cfg = config(vec![repository(
            "source",
            source.path(),
            &["all"],
            invalid_destination,
        )]);

        let error = clone_repo(
            cfg,
            &["all".to_owned()],
            Some(root.to_string_lossy().into_owned()),
        )
        .expect_err("invalid destination should fail before cloning");

        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .expect("invalid destination should preserve io error")
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert!(!root.join(".omni.yaml").exists());
        assert!(!root.join("parent destination").exists());
    }
}

#[test]
fn rejects_duplicate_destinations_before_cloning() {
    let workspace = tempdir().expect("create temporary workspace");
    let root = workspace.path().join("clone root");
    fs::create_dir(&root).expect("create clone root");
    let source_a = init_repository(&workspace, "source-a", "a");
    let source_b = init_repository(&workspace, "source-b", "b");
    let cfg = config(vec![
        repository("A", source_a.path(), &["all"], "same destination"),
        repository("B", source_b.path(), &["all"], "same destination"),
    ]);

    let error = clone_repo(
        cfg,
        &["all".to_owned()],
        Some(root.to_string_lossy().into_owned()),
    )
    .expect_err("duplicate destinations should fail before cloning");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("duplicate destination should preserve io error")
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert!(!root.join("same destination").exists());
    assert!(!root.join(".omni.yaml").exists());
}
