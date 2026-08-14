//! Focused proof for building the Cargo package from a clean locked
//! exact-SHA checkout.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_build::{PackageArtifact, PackageError, build_locked_package};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-build-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn crate_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("src");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"release-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .expect("manifest");
    fs::write(root.join(".gitignore"), "target\n").expect("gitignore");
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> u32 { 42 }\n").expect("lib");
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"release-fixture-{}\", env!(\"CARGO_PKG_VERSION\")); }\n",
    )
    .expect("main");
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {:?}", output);
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {:?}", output);
    String::from_utf8(output.stdout)
        .expect("stdout")
        .trim()
        .to_owned()
}

#[test]
fn a_clean_exact_sha_checkout_packages_successfully() {
    let fixture = fixture_base();
    let root = fixture.path().join("crate");
    crate_fixture(&root);
    git(&root, &["init", "--quiet", "-b", "main"]);
    git(&root, &["config", "user.name", "Release"]);
    git(&root, &["config", "user.email", "release@example.test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "fixture"]);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let artifact = build_locked_package(&root, &head).expect("package");
    assert_eq!(artifact.crate_name, "release-fixture");
    assert_eq!(artifact.version, "0.1.0");
    assert!(!artifact.checksum.is_empty());
    assert!(
        artifact.artifact_path.exists(),
        "the .crate artifact exists"
    );
}

#[test]
fn a_dirty_checkout_is_refused() {
    let fixture = fixture_base();
    let root = fixture.path().join("crate");
    crate_fixture(&root);
    git(&root, &["init", "--quiet", "-b", "main"]);
    git(&root, &["config", "user.name", "Release"]);
    git(&root, &["config", "user.email", "release@example.test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "fixture"]);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> u32 { 43 }\n").expect("dirty");
    let error = build_locked_package(&root, &head).expect_err("dirty");
    assert!(
        matches!(error, PackageError::DirtyCheckout { .. }),
        "{error}"
    );
}

#[test]
fn a_checkout_at_the_wrong_sha_is_refused() {
    let fixture = fixture_base();
    let root = fixture.path().join("crate");
    crate_fixture(&root);
    git(&root, &["init", "--quiet", "-b", "main"]);
    git(&root, &["config", "user.name", "Release"]);
    git(&root, &["config", "user.email", "release@example.test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "fixture"]);
    let wrong = "0000000000000000000000000000000000000000";
    let error = build_locked_package(&root, wrong).expect_err("wrong sha");
    assert!(
        matches!(error, PackageError::CommitMismatch { .. }),
        "{error}"
    );
}

#[test]
fn the_artifact_checksum_is_deterministic() {
    let fixture = fixture_base();
    let root = fixture.path().join("crate");
    crate_fixture(&root);
    git(&root, &["init", "--quiet", "-b", "main"]);
    git(&root, &["config", "user.name", "Release"]);
    git(&root, &["config", "user.email", "release@example.test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "fixture"]);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let first = build_locked_package(&root, &head).expect("first");
    // Packaging again over the same clean checkout yields the same
    // artifact bytes (the .crate is deterministic for locked inputs).
    let second = build_locked_package(&root, &head).expect("second");
    assert_eq!(
        first.checksum, second.checksum,
        "the checksum is deterministic"
    );
}
