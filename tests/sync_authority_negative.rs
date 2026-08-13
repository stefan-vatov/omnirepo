use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::TempDir;

const DESTINATION_SENTINEL: &str = "DESTINATION_ONLY_SOURCE_SENTINEL\n";
const LEGACY_AUTHORITY_MARKERS: &[&str] = &[
    "load_config_default",
    "dirs::home_dir",
    "std::env::current_dir",
    "std::env::var(",
    "option_env!(",
    ".omni.yaml",
    "source_file",
    "template_file",
    "sync_file",
    "RepoConfig",
];

fn command(home: &Path, current_dir: &Path, destination: &Path) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("omnirepo");
    command
        .current_dir(current_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PWD", current_dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("OMNIREPO_CONFIG", current_dir.join("poisoned-config.yaml"))
        .env("OMNIREPO_SOURCE", destination.join("would-be-source.txt"))
        .env(
            "OMNIREPO_SOURCE_FILE",
            destination.join("would-be-source.txt"),
        )
        .env("OMNIREPO_TEMPLATE_ID", "destination-template-id")
        .env(
            "OMNIREPO_TEMPLATE_FILE",
            destination.join("would-be-source.txt"),
        )
        .env("OMNIREPO_DESTINATION", destination)
        .env("OMNIREPO_FILE", destination.join("target.txt"))
        .env("OMNIREPO_TARGET", destination.join("target.txt"));
    command
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn collect(root: &Path, current: &Path, snapshot: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let entries = fs::read_dir(current).expect("read authority fixture directory");
        for entry in entries {
            let path = entry.expect("read authority fixture entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("fixture entry is below its root")
                .to_path_buf();
            if path.is_dir() {
                snapshot.insert(relative, None);
                collect(root, &path, snapshot);
            } else {
                snapshot.insert(
                    relative,
                    Some(fs::read(&path).expect("read authority fixture file")),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    collect(root, root, &mut snapshot);
    snapshot
}

fn legacy_authority_marker(source: &str) -> Option<&'static str> {
    LEGACY_AUTHORITY_MARKERS
        .iter()
        .copied()
        .find(|marker| source.contains(marker))
}

#[test]
fn legacy_invocations_cannot_use_destination_cwd_or_environment_authority() {
    let home = TempDir::new().expect("create poisoned home");
    let current_dir = TempDir::new().expect("create poisoned cwd");
    let destination = current_dir.path().join("destination");
    fs::create_dir_all(destination.join(".omnirepo")).expect("create destination authority dir");
    fs::create_dir_all(current_dir.path().join(".omnirepo")).expect("create cwd authority dir");

    let poisoned_files = [
        home.path().join(".omnirepo.yaml"),
        home.path().join(".omni.yaml"),
        current_dir.path().join(".omnirepo.yaml"),
        current_dir.path().join(".omni.yaml"),
        current_dir.path().join(".omnirepo/source.yaml"),
        destination.join(".omnirepo.yaml"),
        destination.join(".omni.yaml"),
        destination.join(".omnirepo/source.yaml"),
        destination.join("would-be-source.txt"),
        destination.join("target.txt"),
    ];
    for path in &poisoned_files {
        fs::write(path, DESTINATION_SENTINEL).expect("write poisoned authority fixture");
    }

    let destination_source = destination.join("would-be-source.txt");
    let destination_target = destination.join("target.txt");
    let destination_root = destination.to_str().expect("destination is valid UTF-8");
    let source_path = destination_source
        .to_str()
        .expect("source path is valid UTF-8");
    let target_path = destination_target
        .to_str()
        .expect("target path is valid UTF-8");
    let invocations = vec![
        vec![
            "sync".to_owned(),
            "--url".to_owned(),
            "https://example.test/legacy".to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--source-file".to_owned(),
            source_path.to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--template-file".to_owned(),
            "destination-template-id".to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--destination".to_owned(),
            destination_root.to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--file".to_owned(),
            target_path.to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--file".to_owned(),
            target_path.to_owned(),
            "--url".to_owned(),
            "https://example.test/legacy".to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--file".to_owned(),
            target_path.to_owned(),
            "--source-file".to_owned(),
            source_path.to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--file".to_owned(),
            target_path.to_owned(),
            "--template-file".to_owned(),
            "destination-template-id".to_owned(),
        ],
        vec![
            "sync".to_owned(),
            "--file".to_owned(),
            target_path.to_owned(),
            "--destination".to_owned(),
            destination_root.to_owned(),
        ],
    ];

    let before_home = snapshot_files(home.path());
    let before_cwd = snapshot_files(current_dir.path());
    for invocation in invocations {
        let output = command(home.path(), current_dir.path(), &destination)
            .args(&invocation)
            .output()
            .expect("run legacy invocation");
        assert_eq!(
            output.status.code(),
            Some(2),
            "legacy invocation must fail as an argument error: {invocation:?}"
        );
        let mut output_text = String::from_utf8_lossy(&output.stdout).into_owned();
        output_text.push_str(&String::from_utf8_lossy(&output.stderr));
        assert!(
            output_text.contains("unexpected argument"),
            "legacy invocation must report an explicit argument error: {invocation:?}"
        );
        assert!(
            !output_text.contains(DESTINATION_SENTINEL.trim()),
            "legacy invocation must not read or print destination authority: {invocation:?}"
        );
        assert_eq!(
            before_home,
            snapshot_files(home.path()),
            "legacy invocation must not read through a home fallback: {invocation:?}"
        );
        assert_eq!(
            before_cwd,
            snapshot_files(current_dir.path()),
            "legacy invocation must not read through a cwd fallback: {invocation:?}"
        );
    }
}

#[test]
fn static_seam_detects_a_mutated_ambient_fallback() {
    const MUTATED_FALLBACK: &str = r#"
        let candidate = std::env::current_dir().unwrap().join(".omni.yaml");
        let source = std::fs::read_to_string(candidate).unwrap();
    "#;

    assert_eq!(
        legacy_authority_marker(MUTATED_FALLBACK),
        Some("std::env::current_dir"),
        "the seam must detect a cwd fallback mutation"
    );

    let production_file = "src/main.rs";
    let source = fs::read_to_string(env!("CARGO_MANIFEST_DIR").to_owned() + "/" + production_file)
        .expect("read executable public surface");
    assert_eq!(
        legacy_authority_marker(&source),
        None,
        "legacy authority marker remains in {production_file}"
    );
}
