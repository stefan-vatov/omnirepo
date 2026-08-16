//! Focused proof for building and verifying platform binary candidate
//! bundles.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_platform::{
    BundleError, PlatformBundle, build_platform_bundle_for, verify_bundle,
};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-platform-")
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

fn fixture_repo(fixture: &tempfile::TempDir) -> (std::path::PathBuf, String) {
    let root = fixture.path().join("crate");
    crate_fixture(&root);
    git(&root, &["init", "--quiet", "-b", "main"]);
    git(&root, &["config", "user.name", "Release"]);
    git(&root, &["config", "user.email", "release@example.test"]);
    // The locked build needs a committed lockfile.
    let locked = Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&root)
        .output()
        .expect("lockfile");
    assert!(locked.status.success(), "{locked:?}");
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "fixture"]);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    (root, head)
}

#[test]
fn the_local_host_bundle_builds_and_verifies() {
    let fixture = fixture_base();
    let (root, head) = fixture_repo(&fixture);
    let bundle = build_local_bundle(&root, &head).expect("bundle");
    assert!(bundle.binary_path.exists(), "the binary exists");
    assert!(!bundle.checksum.is_empty());
    let verification = verify_bundle(&bundle).expect("verify");
    assert!(verification.help_ok, "the bundle --help works");
    assert!(verification.version_ok, "the bundle --version works");
}

#[test]
fn an_unavailable_cross_target_fails_typed() {
    let fixture = fixture_base();
    let (root, head) = fixture_repo(&fixture);
    // A target that no standard toolchain ships (and no rustup shim can
    // auto-install on a non-rustup rustc) fails typed, never a panic and
    // never a fake bundle.
    let error =
        build_platform_bundle_for(&root, &head, "x86_64-unknown-fuchsia").expect_err("cross");
    assert!(
        matches!(error, BundleError::TargetUnavailable { .. }),
        "{error}"
    );
}

#[test]
fn the_bundle_checksum_is_deterministic() {
    let fixture = fixture_base();
    let (root, head) = fixture_repo(&fixture);
    let first = build_local_bundle(&root, &head).expect("first");
    let second = build_local_bundle(&root, &head).expect("second");
    assert_eq!(
        first.checksum, second.checksum,
        "the bundle checksum is deterministic"
    );
}

fn build_local_bundle(root: &Path, _head: &str) -> Result<PlatformBundle, BundleError> {
    #[cfg(target_os = "linux")]
    let target = "x86_64-unknown-linux-gnu";
    #[cfg(target_os = "macos")]
    let target = "aarch64-apple-darwin";
    build_platform_bundle_for(root, _head, target)
}
