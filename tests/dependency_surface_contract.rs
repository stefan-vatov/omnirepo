//! Contract for the post-binary-conversion dependency surface.
//!
//! This is intentionally RED until the dependency-cleanup owner removes the
//! proven-retired direct crates and makes the executable lockfile explicit.
//! It prevents a later manifest edit from silently restoring the old shared
//! library surface or hiding the lockfile from version control.

use std::fs;
use std::{path::PathBuf, process::Command};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn section<'a>(manifest: &'a str, header: &str) -> &'a str {
    let start = manifest
        .find(header)
        .unwrap_or_else(|| panic!("manifest must contain {header}"));
    let body = &manifest[start + header.len()..];
    body.find("\n[").map(|end| &body[..end]).unwrap_or(body)
}

#[test]
fn retired_direct_dependencies_and_lockfile_ignore_are_absent() {
    let root = repository_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let gitignore = fs::read_to_string(root.join(".gitignore")).expect("read .gitignore");

    let runtime = section(&manifest, "[dependencies]");
    let development = section(&manifest, "[dev-dependencies]");

    for retired in ["dirs", "fern", "log", "once_cell", "proptest"] {
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{retired} ="))),
            "RED: retired direct dependency `{retired}` remains in Cargo.toml"
        );
    }

    for test_only in ["serde", "yaml_serde"] {
        assert!(
            !runtime
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{test_only} ="))),
            "RED: test-only dependency `{test_only}` remains under [dependencies]"
        );
        assert!(
            development
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{test_only} ="))),
            "RED: test-only dependency `{test_only}` must be declared under [dev-dependencies]"
        );
    }

    assert!(
        !manifest
            .lines()
            .any(|line| line.trim() == r#""Cargo.lock""#),
        "RED: the executable workspace must not exclude Cargo.lock from its package"
    );
    assert!(
        !gitignore.lines().any(|line| line.trim() == "Cargo.lock"),
        "RED: the executable workspace must track Cargo.lock"
    );
    assert!(
        !manifest.lines().any(|line| line.trim() == "[features]"),
        "RED: the binary package has no feature surface after legacy removal"
    );
}

#[test]
fn runtime_dependency_allowlist_is_exact_and_lockfile_is_tracked() {
    let root = repository_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let runtime = section(&manifest, "[dependencies]");
    let names = runtime
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name.trim().to_owned())
        .filter(|name| !name.is_empty() && !name.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["clap"],
        "case=runtime-dependency-allowlist unexpected_direct_dependencies={names:?}"
    );

    let tracked = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["ls-files", "--error-unmatch", "Cargo.lock"])
        .current_dir(&root)
        .output()
        .expect("check Cargo.lock tracking");
    assert!(
        tracked.status.success(),
        "case=lockfile-tracking path=Cargo.lock stdout={} stderr={} replay=rtk git ls-files --error-unmatch Cargo.lock",
        String::from_utf8_lossy(&tracked.stdout),
        String::from_utf8_lossy(&tracked.stderr)
    );
}
