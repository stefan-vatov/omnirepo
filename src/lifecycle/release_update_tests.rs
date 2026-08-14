//! Supported predecessor update and migration-guidance verification:
//! a 0.8.x predecessor home updates to the constitutional release
//! without implicit migration, and the migration guidance stays
//! reachable.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-update-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn omnirepo_binary() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug/omnirepo")
        .to_path_buf()
}

fn run(binary: &Path, home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary)
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run")
}

#[test]
fn a_predecessor_home_is_never_migrated_implicitly() {
    let fixture = fixture_base();
    let home = fixture.path().join("predecessor-home");
    fs::create_dir_all(home.join(".omnirepo")).expect("dir");
    // The 0.8.x-era legacy config file exists.
    let legacy = home.join(".omnirepo/config.yaml");
    fs::write(
        &legacy,
        "version: 1\nrepositories:\n  - id: old\n    path: /srv/old\n",
    )
    .expect("legacy config");
    let binary = omnirepo_binary();
    let output = run(&binary, &home, &["sync"]);
    // The sync never rewrites or migrates the existing authority.
    assert_eq!(
        fs::read_to_string(&legacy).expect("legacy untouched"),
        "version: 1\nrepositories:\n  - id: old\n    path: /srv/old\n"
    );
    // The legacy file is valid canonical config: the fleet is honored.
    // (The destination does not exist, so the run fails typed instead
    // of migrating anything.)
    assert!(
        output.status.code() != Some(0),
        "no silent success over a broken fleet"
    );
}

#[test]
fn the_migration_guidance_is_reachable() {
    for doc in [
        "docs/breaking-guidance.md",
        "docs/breaks-inventory.md",
        "docs/reference.md",
        "docs/quickstart.md",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(doc);
        assert!(path.exists(), "guidance {doc} is reachable");
    }
    // The README links to the guidance.
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("readme");
    assert!(readme.contains("docs/breaking-guidance.md"));
}

#[test]
fn the_update_preserves_the_canonical_configuration() {
    let fixture = fixture_base();
    let home = fixture.path().join("update-home");
    fs::create_dir_all(home.join(".omnirepo")).expect("dir");
    let canonical = home.join(".omnirepo/config.yaml");
    let content = "version: 1\nrepositories: []\n";
    fs::write(&canonical, content).expect("canonical config");
    let binary = omnirepo_binary();
    let output = run(&binary, &home, &["sync"]);
    assert_eq!(output.status.code(), Some(0), "empty fleet succeeds");
    // The canonical config survives the update byte-exactly.
    assert_eq!(
        fs::read_to_string(&canonical).expect("preserved"),
        content,
        "the canonical configuration is preserved"
    );
}

#[test]
fn the_predecessor_migration_guidance_is_actionable_per_break() {
    let guidance =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/breaking-guidance.md"))
            .expect("guidance");
    assert!(guidance.contains("**How to migrate.**"));
    assert!(guidance.contains("**If you do not migrate.**"));
    assert!(guidance.contains("Automated migration is declined"));
}
