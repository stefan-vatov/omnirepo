//! Two-process overlap, kill/restart, and stale-lock recovery fixtures.

#![allow(dead_code, unused_imports)]

use super::SourceLock;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Fork-helper dispatch: when the environment names a helper mode, run it in
/// the child process and exit; otherwise return false so the test body runs.
fn fork_helper() -> bool {
    let Some(mode) = std::env::var_os("OMNIREPO_FORK_HELPER") else {
        return false;
    };
    let cache = PathBuf::from(std::env::var("OMNIREPO_FORK_CACHE").expect("fork cache"));
    match mode.to_str() {
        Some("hold-source-lock") => {
            let _lock = SourceLock::acquire(&cache, "upstream").expect("child lock");
            // Held until the parent kills us; the bound only guards a broken
            // parent (the parent always kills within the test).
            std::thread::sleep(Duration::from_secs(60));
        }
        _ => panic!("unknown fork helper mode"),
    }
    std::process::exit(0);
}

fn spawn_holder(cache: &Path) -> std::process::Child {
    let executable = std::env::current_exe().expect("test binary");
    Command::new(executable)
        .args([
            "--exact",
            "source::acquisition::acquisition_concurrency_tests::live_owner_lock_is_not_reclaimed",
        ])
        .env("OMNIREPO_FORK_HELPER", "hold-source-lock")
        .env("OMNIREPO_FORK_CACHE", cache)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn holder")
}

#[test]
fn live_owner_lock_is_not_reclaimed() {
    if fork_helper() {
        return;
    }
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("source-lock-live-")
        .tempdir_in(&base)
        .expect("fixture");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("cache");
    let mut child = spawn_holder(&cache);
    // Give the child a moment to take the lock.
    std::thread::sleep(Duration::from_millis(400));
    let error = SourceLock::acquire_with_wait(&cache, "upstream", Duration::from_millis(300))
        .expect_err("live lock must not be reclaimed");
    assert!(format!("{error}").contains("timed out waiting"), "{error}");
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn killed_owner_lock_is_reclaimed() {
    if fork_helper() {
        return;
    }
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("source-lock-stale-")
        .tempdir_in(&base)
        .expect("fixture");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("cache");
    let mut child = spawn_holder(&cache);
    std::thread::sleep(Duration::from_millis(400));
    // SIGKILL the holder: the kernel releases its lock.
    let _ = child.kill();
    let _ = child.wait();
    let lock = SourceLock::acquire_with_wait(&cache, "upstream", Duration::from_secs(5))
        .expect("stale lock reclaimed");
    let lock_path = cache.join(".upstream.lock");
    assert!(lock_path.is_file(), "stable lock inode");
    drop(lock);
    SourceLock::acquire_with_wait(&cache, "upstream", Duration::from_millis(100))
        .expect("released lock can be acquired");
}

#[test]
fn stale_pid_file_does_not_impersonate_a_live_lock_owner() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("source-lock-pid-reuse-")
        .tempdir_in(&base)
        .expect("fixture");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("cache");
    fs::write(cache.join(".upstream.lock"), std::process::id().to_string())
        .expect("write residue with a reused PID");

    SourceLock::acquire_with_wait(&cache, "upstream", Duration::from_millis(100))
        .expect("residue without a kernel lock has no owner");
}
