//! Focused proof for clean fresh-install and channel-specific candidate
//! verification.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_verify::{
    Channel, InstallVerification, VerifyError, verify_channel, verify_fresh_install,
};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-verify-")
        .tempdir_in(&base)
        .expect("fixture")
}

/// The freshly built binary in this test run's own target directory
/// (`<target>/debug/deps/<test>` → `<target>/debug/omnirepo`), so the
/// test never depends on a stale or cache-restored `target/debug` and
/// works under profile-specific target directories (cargo-llvm-cov).
fn omnirepo_binary() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("test executable");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("omnirepo")
}

#[test]
fn a_fresh_install_in_a_clean_home_verifies_the_full_surface() {
    let fixture = fixture_base();
    let clean_home = fixture.path().join("clean-home");
    fs::create_dir_all(&clean_home).expect("home");
    let verification = verify_fresh_install(&omnirepo_binary(), &clean_home).expect("verify");
    assert!(verification.help_ok, "help works after a fresh install");
    assert!(
        verification.version_ok,
        "version works after a fresh install"
    );
    assert!(verification.sync_empty_ok, "an empty-fleet sync works");
    // The fresh install left a durable record in the clean home.
    let runs = clean_home.join(".omnirepo/runs");
    let records = fs::read_dir(&runs).expect("runs").count();
    assert_eq!(records, 1, "exactly one record: {records}");
}

#[test]
fn a_missing_binary_fails_typed() {
    let fixture = fixture_base();
    let clean_home = fixture.path().join("clean-home");
    fs::create_dir_all(&clean_home).expect("home");
    let error = verify_fresh_install(&fixture.path().join("missing"), &clean_home)
        .expect_err("missing binary");
    assert!(matches!(error, VerifyError::Binary { .. }), "{error}");
}

#[test]
fn the_non_public_channel_never_publishes() {
    // A non-public release candidate is verified locally and never
    // published to any channel.
    let outcome = verify_channel("0.9.0-rc.1", Channel::NonPublic).expect("verify");
    assert!(outcome.local_verified);
    assert!(
        !outcome.published,
        "a non-public candidate is never published"
    );
}

#[test]
fn the_public_channel_requires_a_promotion_decision() {
    let outcome = verify_channel("0.9.0", Channel::Public).expect("verify");
    assert!(outcome.local_verified);
    // A public release requires the explicit promotion gate; without it
    // nothing is published.
    assert!(!outcome.published);
}
