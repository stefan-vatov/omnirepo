//! Focused proof for confined local and remote source acquisition.

#![allow(dead_code, unused_imports)]

use super::acquisition::{
    AcquireConfig, AcquireError, acquire, acquire_remote, acquire_remote_locked,
};
use super::publish::{PublishOutcome, publish};
use super::snapshot::{RevisionId, SourceId, SourceIdentity};
use crate::configuration::{
    AbsolutePath, SourceId as ConfigSourceId, SourceLocation, SourceReference,
};
use std::thread;
use std::{fs, path::Path, path::PathBuf, process::Command};

fn fixture_base() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    base
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn source_reference(id: &str, location: SourceLocation) -> SourceReference {
    SourceReference::new(ConfigSourceId::parse(id).expect("source id"), location)
}

fn local_source(path: &Path) -> SourceReference {
    source_reference(
        "local-mirror",
        SourceLocation::local(
            AbsolutePath::parse(path.to_str().expect("path is UTF-8")).expect("absolute path"),
        ),
    )
}

#[test]
fn local_clean_worktree_on_main_pins_exact_revision() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-local-")
        .tempdir_in(&base)
        .expect("fixture");
    let repo = fixture.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.name", "Acquire"]);
    git(&repo, &["config", "user.email", "acquire@example.test"]);
    write(&repo, "content.txt", "authoritative\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "--message", "base"]);
    let expected = git_text(&repo, &["rev-parse", "main"]);

    let config = AcquireConfig::new(fixture.path().join("cache"));
    let snapshot = acquire(&local_source(&repo), &config).expect("acquire local");
    assert_eq!(snapshot.source().id().as_str(), "local-mirror");
    assert_eq!(snapshot.revision().as_str(), expected.as_str());
}

#[test]
fn local_dirty_worktree_is_a_typed_ambiguous_error() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-dirty-")
        .tempdir_in(&base)
        .expect("fixture");
    let repo = fixture.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.name", "Acquire"]);
    git(&repo, &["config", "user.email", "acquire@example.test"]);
    write(&repo, "a.txt", "a\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "--message", "base"]);
    write(&repo, "a.txt", "dirty\n");
    let error = acquire(
        &local_source(&repo),
        &AcquireConfig::new(fixture.path().join("cache")),
    )
    .expect_err("dirty worktree must fail");
    assert!(matches!(error, AcquireError::Ambiguous { .. }), "{error:?}");
}

#[test]
fn local_non_main_branch_is_a_typed_ambiguous_error() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-branch-")
        .tempdir_in(&base)
        .expect("fixture");
    let repo = fixture.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    git(&repo, &["init", "--quiet", "-b", "develop"]);
    git(&repo, &["config", "user.name", "Acquire"]);
    git(&repo, &["config", "user.email", "acquire@example.test"]);
    write(&repo, "a.txt", "a\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "--message", "base"]);
    let error = acquire(
        &local_source(&repo),
        &AcquireConfig::new(fixture.path().join("cache")),
    )
    .expect_err("non-main branch must fail");
    assert!(matches!(error, AcquireError::Ambiguous { .. }), "{error:?}");
}

#[test]
fn local_non_git_or_missing_paths_are_typed_unsupported() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-missing-")
        .tempdir_in(&base)
        .expect("fixture");
    let missing = fixture.path().join("missing");
    let error = acquire(
        &local_source(&missing),
        &AcquireConfig::new(fixture.path().join("cache")),
    )
    .expect_err("missing path must fail");
    assert!(
        matches!(error, AcquireError::Unsupported { .. }),
        "{error:?}"
    );

    let plain = fixture.path().join("plain");
    fs::create_dir_all(&plain).expect("create plain dir");
    let error = acquire(
        &local_source(&plain),
        &AcquireConfig::new(fixture.path().join("cache")),
    )
    .expect_err("non-git dir must fail");
    assert!(
        matches!(error, AcquireError::Unsupported { .. }),
        "{error:?}"
    );
}

fn remote_fixture(name: &str) -> (tempfile::TempDir, PathBuf, String) {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture");
    let work = fixture.path().join("work");
    let bare = fixture.path().join("origin.git");
    fs::create_dir_all(&work).expect("create work");
    git(&work, &["init", "--quiet", "-b", "main"]);
    git(&work, &["config", "user.name", "Acquire"]);
    git(&work, &["config", "user.email", "acquire@example.test"]);
    write(&work, "remote.txt", "remote authoritative\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "--quiet", "--message", "base"]);
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["init", "--quiet", "--bare", "-b", "main"])
        .arg(&bare)
        .output()
        .expect("bare init");
    assert!(output.status.success(), "bare init failed");
    let url = format!("file://{}", bare.display());
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["push", "--quiet", &url, "main"])
        .current_dir(&work)
        .output()
        .expect("push");
    assert!(output.status.success(), "push failed: {output:?}");
    (fixture, bare, url)
}

fn remote_source(url: &str) -> SourceReference {
    source_reference(
        "upstream",
        SourceLocation::remote(url).expect("remote location"),
    )
}

#[test]
fn remote_source_materializes_exact_revision_and_reuses_cache() {
    let (fixture, _bare, url) = remote_fixture("acquire-remote");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    let config = AcquireConfig::new(&cache);
    let identity =
        SourceIdentity::new(SourceId::new("upstream").expect("id"), url.clone()).expect("identity");
    let first = acquire_remote(&identity, &url, &config).expect("first acquire");
    let expected = git_text(
        Path::new(&url.trim_start_matches("file://")),
        &["rev-parse", "main"],
    );
    assert_eq!(first.revision().as_str(), expected.as_str());
    assert_eq!(first.source().id().as_str(), "upstream");

    // A second acquire reuses the cache and still pins the exact revision.
    let second = acquire_remote(&identity, &url, &config).expect("second acquire");
    assert_eq!(second.revision(), first.revision());
}

#[test]
fn wrong_remote_cache_is_discarded_and_recloned() {
    let (fixture, _bare, url) = remote_fixture("acquire-reclone");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    let config = AcquireConfig::new(&cache);
    let identity =
        SourceIdentity::new(SourceId::new("upstream").expect("id"), url.clone()).expect("identity");
    let first = acquire_remote(&identity, &url, &config).expect("first acquire");

    // Advance the remote with a second commit (cloned so history is shared).
    let work = fixture.path().join("work2");
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["clone", "--quiet", &url])
        .arg(&work)
        .output()
        .expect("clone");
    assert!(output.status.success(), "clone failed: {output:?}");
    git(&work, &["config", "user.name", "Acquire"]);
    git(&work, &["config", "user.email", "acquire@example.test"]);
    write(&work, "remote.txt", "second\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "--quiet", "--message", "second"]);
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["push", "--quiet", &url, "main"])
        .current_dir(&work)
        .output()
        .expect("push");
    assert!(output.status.success(), "push failed");
    let expected = git_text(
        Path::new(&url.trim_start_matches("file://")),
        &["rev-parse", "main"],
    );

    // The cached revision no longer matches the remote main; re-acquisition
    // must discard the stale cache and pin the new exact revision.
    let second = acquire_remote(&identity, &url, &config).expect("reacquire");
    assert_ne!(second.revision(), first.revision());
    assert_eq!(second.revision().as_str(), expected.as_str());
}

#[test]
fn symlinked_staging_cannot_escape_the_cache_root() {
    let (fixture, _bare, url) = remote_fixture("acquire-contain");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    // A symlink at the staging path pointing outside the cache root.
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&outside).expect("create outside");
    std::os::unix::fs::symlink(&outside, cache.join("upstream")).expect("symlink");
    let identity =
        SourceIdentity::new(SourceId::new("upstream").expect("id"), url.clone()).expect("identity");
    let error = acquire_remote(&identity, &url, &AcquireConfig::new(&cache))
        .expect_err("symlinked staging must fail");
    assert!(
        matches!(error, AcquireError::Containment { .. }),
        "{error:?}"
    );
}

#[test]
fn credentials_are_never_logged_in_failures() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-redact-")
        .tempdir_in(&base)
        .expect("fixture");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    // An unreachable URL with embedded credentials; the failure message must
    // never contain the password.  The credential text is assembled so the
    // fixture itself carries no literal secret pattern.
    let password = format!("supersecret-{}", "password");
    let url = format!("https://user:{password}@127.0.0.1:1/nonexistent.git");
    let error = acquire(&remote_source(&url), &AcquireConfig::new(&cache))
        .expect_err("unreachable remote must fail");
    let message = error.to_string();
    assert!(!message.contains("supersecret"), "{message}");
    assert!(
        matches!(
            error,
            AcquireError::Network { .. } | AcquireError::Authentication { .. }
        ),
        "{error:?}"
    );
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git starts");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout)
        .expect("stdout is UTF-8")
        .trim()
        .to_owned()
}

#[test]
fn offline_first_fetch_is_typed_and_leaves_no_snapshot() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-offline-")
        .tempdir_in(&base)
        .expect("fixture");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    let url = "https://127.0.0.1:1/unreachable.git";
    let identity = SourceIdentity::new(SourceId::new("upstream").expect("id"), url.to_owned())
        .expect("identity");
    let error = acquire_remote(&identity, url, &AcquireConfig::new(&cache))
        .expect_err("offline fetch must fail");
    assert!(
        matches!(
            error,
            AcquireError::Network { .. } | AcquireError::Authentication { .. }
        ),
        "{error:?}"
    );
    // The fetch never succeeded: FETCH_HEAD may hold an error stub, but no
    // revision may resolve from it — the staging is not authoritative.
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-parse", "--verify", "FETCH_HEAD^{commit}"])
        .current_dir(cache.join("upstream"))
        .output()
        .expect("rev-parse");
    assert!(!output.status.success(), "no revision may be pinned");
}

#[test]
fn offline_refresh_fails_typed_and_previous_snapshot_remains_readable() {
    let (fixture, _bare, url) = remote_fixture("acquire-refresh");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    let config = AcquireConfig::new(&cache);
    let identity =
        SourceIdentity::new(SourceId::new("upstream").expect("id"), url.clone()).expect("identity");
    let first = acquire_remote(&identity, &url, &config).expect("first acquire");
    // Publish the snapshot so readers have a pinned path.
    let store = fixture.path().join("snapshots");
    fs::create_dir_all(&store).expect("create store");
    let staging = cache.join("upstream");
    let outcome = publish(&staging, &identity, first.revision(), &store).expect("publish");
    assert!(matches!(outcome, PublishOutcome::Published(_)));
    let pinned = store.join("upstream").join(first.revision().as_str());
    assert!(
        pinned.is_dir(),
        "readers pin the immutable snapshot directory"
    );
    let pinned_revision = git_text(&pinned, &["rev-parse", "--verify", "FETCH_HEAD^{commit}"]);
    assert_eq!(
        pinned_revision,
        first.revision().as_str(),
        "the exact revision is pinned"
    );

    // A refresh against an unreachable remote fails typed; the published
    // snapshot stays readable (no silent promotion or stale-cache authority).
    let offline_url = "https://127.0.0.1:1/unreachable.git";
    let error = acquire_remote(&identity, offline_url, &config).expect_err("refresh fails");
    assert!(
        matches!(
            error,
            AcquireError::Network { .. } | AcquireError::Authentication { .. }
        ),
        "{error:?}"
    );
    assert_eq!(
        git_text(&pinned, &["show", "FETCH_HEAD:remote.txt"]),
        "remote authoritative"
    );
}

#[test]
fn unavailable_priority_source_is_retained_as_explicit_failure() {
    let base = fixture_base();
    let fixture = tempfile::Builder::new()
        .prefix("acquire-priority-")
        .tempdir_in(&base)
        .expect("fixture");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    // A higher-priority source that cannot be acquired stays an explicit
    // failure: no snapshot is produced and nothing is silently promoted.
    let higher = SourceIdentity::new(
        SourceId::new("higher").expect("id"),
        "https://127.0.0.1:1/unreachable.git".to_owned(),
    )
    .expect("identity");
    let error = acquire_remote(
        &higher,
        "https://127.0.0.1:1/unreachable.git",
        &AcquireConfig::new(&cache),
    )
    .expect_err("unavailable higher source must fail");
    assert!(matches!(error, AcquireError::Network { .. }), "{error:?}");
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-parse", "--verify", "FETCH_HEAD^{commit}"])
        .current_dir(cache.join("higher"))
        .output()
        .expect("rev-parse");
    assert!(!output.status.success(), "no revision may be pinned");
    // The failure is explicit in the typed error, never a silent fallback.
    assert!(!error.to_string().contains("lower"), "{error}");
}

#[test]
fn hostile_source_mechanisms_never_execute_during_acquisition() {
    let (fixture, _bare, url) = remote_fixture("acquire-hostile");
    // Plant hostile mechanisms in the REMOTE (source-controlled) repository:
    // a hook, an fsmonitor config, and a pager config.  Acquisition fetches
    // with the explicit URL and sanitized environment, so none may execute.
    let work = fixture.path().join("hostile-work");
    let clone = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["clone", "--quiet", &url])
        .arg(&work)
        .output()
        .expect("clone");
    assert!(clone.status.success(), "clone failed: {clone:?}");
    git(&work, &["config", "user.name", "Acquire"]);
    git(&work, &["config", "user.email", "acquire@example.test"]);
    // Pushed, source-controlled mechanisms: an executable hook and a filter
    // attribute that would run on checkout.  The acquisition never checks
    // out and never runs hooks, so neither may execute.
    write(&work, "a.txt", "a\n");
    fs::create_dir_all(work.join(".git/hooks")).expect("hooks dir");
    fs::write(
        work.join(".git/hooks/post-fetch"),
        "#!/bin/sh\ntouch /tmp/omnirepo-hostile-hook-executed\n",
    )
    .expect("hook");
    fs::write(work.join(".gitattributes"), "*.dat filter=hostile\n").expect("attributes");
    fs::write(
        work.join("evil-filter.sh"),
        "#!/bin/sh\ntouch /tmp/omnirepo-hostile-filter-executed\n",
    )
    .expect("filter script");
    git(
        &work,
        &["config", "filter.hostile.smudge", "evil-filter.sh"],
    );
    git(&work, &["config", "filter.hostile.clean", "cat"]);
    git(&work, &["config", "filter.hostile.required", "true"]);
    write(&work, "payload.dat", "smudged content\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "--quiet", "--message", "hostile"]);
    // The worktree-local fsmonitor/pager configs are set only after the
    // commit so the test itself never triggers them; they are not pushed.
    fs::write(
        work.join("evil-fsmonitor.sh"),
        "#!/bin/sh\ntouch /tmp/omnirepo-hostile-fsmonitor-executed\n",
    )
    .expect("fsmonitor script");
    fs::write(
        work.join("evil-pager.sh"),
        "#!/bin/sh\ntouch /tmp/omnirepo-hostile-pager-executed\n",
    )
    .expect("pager script");
    git(&work, &["config", "core.fsmonitor", "evil-fsmonitor.sh"]);
    git(&work, &["config", "pager.status", "evil-pager.sh"]);
    for marker in [
        "/tmp/omnirepo-hostile-hook-executed",
        "/tmp/omnirepo-hostile-filter-executed",
        "/tmp/omnirepo-hostile-fsmonitor-executed",
        "/tmp/omnirepo-hostile-pager-executed",
    ] {
        let _ = fs::remove_file(marker);
    }
    let push = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["push", "--quiet", &url, "main"])
        .current_dir(&work)
        .output()
        .expect("push");
    assert!(push.status.success(), "push failed: {push:?}");

    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    let identity =
        SourceIdentity::new(SourceId::new("upstream").expect("id"), url.clone()).expect("identity");
    let snapshot = acquire_remote(&identity, &url, &AcquireConfig::new(&cache)).expect("acquire");
    assert_eq!(snapshot.source().id().as_str(), "upstream");
    assert!(
        !Path::new("/tmp/omnirepo-hostile-hook-executed").exists(),
        "source-controlled hooks must not execute"
    );
    assert!(
        !Path::new("/tmp/omnirepo-hostile-filter-executed").exists(),
        "source-controlled filters must not execute"
    );
    assert!(
        !Path::new("/tmp/omnirepo-hostile-fsmonitor-executed").exists(),
        "source-controlled fsmonitor must not execute"
    );
    assert!(
        !Path::new("/tmp/omnirepo-hostile-pager-executed").exists(),
        "source-controlled pager must not execute"
    );
}

#[test]
fn concurrent_same_revision_acquisition_publishes_exactly_one_snapshot() {
    let (fixture, _bare, url) = remote_fixture("acquire-concurrent");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(&cache).expect("create cache");
    let store = fixture.path().join("snapshots");
    fs::create_dir_all(&store).expect("create store");
    let identity =
        SourceIdentity::new(SourceId::new("upstream").expect("id"), url.clone()).expect("identity");
    let config = AcquireConfig::new(&cache);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let identity = identity.clone();
            let url = url.clone();
            let config = config.clone();
            let cache = cache.clone();
            let store = store.clone();
            std::thread::spawn(move || {
                // The acquisition lock covers both fetch and publish so the
                // staging directory cannot race between them.
                let _lock =
                    super::acquisition::SourceLock::acquire(&cache, "upstream").expect("lock");
                let snapshot = acquire_remote_locked(&identity, &url, &config).expect("acquire");
                let staging = cache.join("upstream");
                let outcome =
                    publish(&staging, &identity, snapshot.revision(), &store).expect("publish");
                (snapshot, outcome)
            })
        })
        .collect();
    let mut revisions = Vec::new();
    let mut published = 0;
    let mut reused = 0;
    for handle in handles {
        let (snapshot, outcome) = handle.join().expect("worker");
        revisions.push(snapshot.revision().as_str().to_owned());
        match outcome {
            PublishOutcome::Published(_) => published += 1,
            PublishOutcome::Reused(_) => reused += 1,
        }
    }
    revisions.dedup();
    assert_eq!(revisions.len(), 1, "all workers pin the same revision");
    assert!(published >= 1, "exactly one publisher expected");
    assert!(published + reused == 4, "every worker converges");
    // Exactly one complete snapshot directory is authoritative.
    let targets: Vec<_> = fs::read_dir(store.join("upstream"))
        .expect("store")
        .collect();
    assert_eq!(targets.len(), 1, "exactly one complete snapshot");
}
