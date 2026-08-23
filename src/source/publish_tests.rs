//! Focused proof for atomic snapshot publication and reader pinning.

#![allow(dead_code, unused_imports)]

use super::publish::{PublishError, PublishOutcome, publish};
use super::snapshot::{RevisionId, SourceId, SourceIdentity};
use std::{fs, path::Path, path::PathBuf};

fn fixture() -> (tempfile::TempDir, PathBuf, SourceIdentity, RevisionId) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("publish-home-")
        .tempdir_in(&base)
        .expect("create publish fixture");
    let store = fixture.path().join("snapshots");
    fs::create_dir_all(&store).expect("create store");
    let source = SourceIdentity::new(
        SourceId::new("upstream").expect("id"),
        "https://example.com/repo.git",
    )
    .expect("source identity");
    let revision = RevisionId::new("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef").expect("revision");
    (fixture, store, source, revision)
}

fn write_staging(root: &Path, name: &str, content: &str) -> PathBuf {
    let staging = root.join(name);
    fs::create_dir_all(&staging).expect("create staging");
    fs::write(staging.join("managed.txt"), content).expect("write staging content");
    staging
}

#[test]
fn complete_staging_publishes_atomically_with_exact_contents() {
    let (fixture, store, source, revision) = fixture();
    let staging = write_staging(fixture.path(), "staging", "authoritative bytes\n");
    let outcome = publish(&staging, &source, &revision, &store).expect("publish");
    let PublishOutcome::Published(snapshot) = outcome else {
        panic!("expected published");
    };
    let target = store.join("upstream").join(revision.as_str());
    assert!(target.is_dir());
    assert_eq!(
        fs::read_to_string(target.join("managed.txt")).expect("content"),
        "authoritative bytes\n"
    );
    assert_eq!(snapshot.source().id().as_str(), "upstream");
    assert_eq!(snapshot.revision(), &revision);
    // Readers pin by the snapshot's cache path.
    assert_eq!(snapshot.cache().as_str(), target.display().to_string());
}

#[test]
fn staging_hard_link_cannot_make_a_published_snapshot_mutable() {
    let (fixture, store, source, revision) = fixture();
    let outside = fixture.path().join("outside.txt");
    fs::write(&outside, "original\n").expect("write outside file");
    let staging = fixture.path().join("staging");
    let nested = staging.join("nested");
    fs::create_dir_all(&nested).expect("create staging");
    fs::hard_link(&outside, nested.join("managed.txt")).expect("hard-link staging file");

    let error = publish(&staging, &source, &revision, &store)
        .expect_err("hard-linked staging file must fail");

    assert!(matches!(error, PublishError::InvalidStaging { .. }));
    assert!(staging.is_dir(), "rejected staging stays outside the store");
    assert!(
        !store.join("upstream").exists(),
        "invalid staging must not create publication state"
    );
    fs::write(&outside, "changed outside\n").expect("mutate outside file");
    assert_eq!(
        fs::read_to_string(nested.join("managed.txt")).expect("read staging link"),
        "changed outside\n",
        "the rejected alias demonstrates why it cannot become immutable authority"
    );
}

#[test]
fn repeated_publication_of_the_same_revision_reuses() {
    let (fixture, store, source, revision) = fixture();
    let first = write_staging(fixture.path(), "staging-a", "version one\n");
    publish(&first, &source, &revision, &store).expect("first publish");
    // A second complete staging for the same revision must reuse, never
    // overwrite the authoritative bytes.
    let second = write_staging(fixture.path(), "staging-b", "version two\n");
    let outcome = publish(&second, &source, &revision, &store).expect("second publish");
    let PublishOutcome::Reused(_snapshot) = outcome else {
        panic!("expected reused");
    };
    let target = store.join("upstream").join(revision.as_str());
    assert_eq!(
        fs::read_to_string(target.join("managed.txt")).expect("content"),
        "version one\n",
        "the first complete snapshot stays authoritative"
    );
    assert!(!second.exists(), "the duplicate staging is discarded");
}

#[test]
fn distinct_revisions_publish_independently() {
    let (fixture, store, source, revision) = fixture();
    let first = write_staging(fixture.path(), "staging-a", "a\n");
    publish(&first, &source, &revision, &store).expect("first");
    let second_revision =
        RevisionId::new("1111111111111111111111111111111111111111").expect("revision");
    let second = write_staging(fixture.path(), "staging-b", "b\n");
    publish(&second, &source, &second_revision, &store).expect("second");
    assert!(store.join("upstream").join(revision.as_str()).is_dir());
    assert!(
        store
            .join("upstream")
            .join(second_revision.as_str())
            .is_dir()
    );
}

#[test]
fn interrupted_publication_leaves_no_authoritative_partial() {
    let (fixture, store, source, revision) = fixture();
    // A staging tree that never gets published is not authoritative and must
    // not appear in the store at all.
    let staging = write_staging(fixture.path(), "staging", "partial\n");
    assert!(staging.is_dir());
    assert!(
        !store.join("upstream").exists(),
        "no partial snapshot may appear"
    );
    // And a complete publish after the interruption still lands exactly.
    let outcome = publish(&staging, &source, &revision, &store).expect("publish");
    assert!(matches!(outcome, PublishOutcome::Published(_)));
    assert!(store.join("upstream").join(revision.as_str()).is_dir());
}

#[test]
fn invalid_staging_and_conflicting_targets_are_typed() {
    let (fixture, store, source, revision) = fixture();
    // A symlinked staging is never accepted.
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&outside).expect("create outside");
    let link = fixture.path().join("staging-link");
    std::os::unix::fs::symlink(&outside, &link).expect("symlink");
    let error = publish(&link, &source, &revision, &store).expect_err("symlink staging fails");
    assert!(
        matches!(error, PublishError::InvalidStaging { .. }),
        "{error:?}"
    );

    // A pre-existing non-directory target cannot be reused and is never
    // overwritten.
    let target = store.join("upstream").join(revision.as_str());
    fs::create_dir_all(target.parent().expect("parent")).expect("parent");
    fs::write(&target, "not a directory").expect("write conflicting target");
    let staging = write_staging(fixture.path(), "staging-b", "content\n");
    let error = publish(&staging, &source, &revision, &store).expect_err("conflict fails");
    assert!(
        matches!(error, PublishError::ConflictingTarget { .. }),
        "{error:?}"
    );
    assert_eq!(
        fs::read_to_string(&target).expect("unchanged"),
        "not a directory"
    );
}
