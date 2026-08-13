//! Focused proof for confined local and remote source acquisition.

#![allow(dead_code, unused_imports)]

use super::acquisition::{AcquireConfig, AcquireError, acquire, acquire_remote};
use super::snapshot::{RevisionId, SourceId, SourceIdentity};
use crate::configuration::{
    AbsolutePath, SourceId as ConfigSourceId, SourceLocation, SourceReference,
};
use std::{fs, path::Path, path::PathBuf, process::Command};

fn fixture_base() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    base
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
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
        .args(["init", "--quiet", "--bare", "-b", "main"])
        .arg(&bare)
        .output()
        .expect("bare init");
    assert!(output.status.success(), "bare init failed");
    let url = format!("file://{}", bare.display());
    let output = Command::new("git")
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
