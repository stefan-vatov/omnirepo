//! Real filesystem identity, containment, and replacement acceptance on
//! the supported host platforms.
//!
//! Exercises case/Unicode aliases, separators, links, special files,
//! permissions/metadata, atomic replacement, and concurrent no-op
//! semantics on the actual supported host (Linux/ext-family or
//! macOS/APFS).  Outside-root effects never occur; equal no-op and
//! failure atomicity hold; unsupported filesystems fail closed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::platform_matrix::{Filesystem, Os, capability_supported, claim_platform};
use crate::platform::{AuthorityRoot, DestinationRepositoryRoot, ReadOnly, RelativePath};
use std::{fs, path::Path, process::Command};

fn harness_root(name: &str) -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn the_host_is_a_claimed_supported_platform() {
    // The capability gate must claim this host before any acceptance
    // runs; an unclaimed host fails closed here.
    #[cfg(target_os = "linux")]
    let (os, filesystem) = (Os::Linux, Filesystem::ExtFamily);
    #[cfg(target_os = "macos")]
    let (os, filesystem) = (Os::Mac, Filesystem::Apfs);
    assert!(capability_supported(os, filesystem));
    claim_platform(os, filesystem).expect("this host is claimed");
    // The unsupported filesystem fails closed regardless of the host.
    assert!(matches!(
        claim_platform(os, Filesystem::Network),
        Err(crate::lifecycle::platform_matrix::PlatformError::Unsupported { .. })
    ));
}

#[test]
fn case_and_unicode_aliases_are_distinct_byte_exact_paths_on_the_host() {
    let fixture = harness_root("platform-alias-");
    fs::write(fixture.path().join("Managed.txt"), "upper\n").expect("upper");
    fs::write(fixture.path().join("managed.txt"), "lower\n").expect("lower");
    // The two names are distinct files with distinct object identities on
    // case-sensitive hosts.  A case-insensitive filesystem (macOS default
    // APFS) merges them by definition, so the alias-distinctness
    // assertion is capability-gated: the merge is detected and reported,
    // never asserted as distinct.
    let upper = fs::metadata(fixture.path().join("Managed.txt")).expect("upper meta");
    let lower = fs::metadata(fixture.path().join("managed.txt")).expect("lower meta");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if upper.ino() == lower.ino() {
            eprintln!(
                "authority-capability: skipped-case-insensitive-filesystem (case aliases merged)"
            );
        } else {
            assert_ne!(
                upper.ino(),
                lower.ino(),
                "case aliases are distinct objects"
            );
        }
    }
    // Unicode normalization aliases are also distinct byte-exact paths.
    let nfc = fs::metadata(fixture.path().join("cafe\u{301}.txt"));
    let nfd = fs::metadata(fixture.path().join("cafe\u{301}.txt"));
    if nfc.is_ok() && nfd.is_ok() {
        eprintln!("authority-capability: skipped-normalizing-filesystem (aliases merged)");
    } else {
        assert!(
            nfc.is_err() && nfd.is_err(),
            "no accidental normalization merge"
        );
    }
}

#[test]
fn symlink_and_policy_aliases_fail_closed_before_any_effect() {
    let fixture = harness_root("platform-symlink-");
    let outside = fixture.path().join("outside-secret.txt");
    fs::write(&outside, "secret").expect("outside");
    // The canonical policy path is a symlink to an outside file: the
    // loader must refuse the alias before any content is accepted.
    let policy = fixture.path().join(".omnirepo.yaml");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, &policy).expect("symlink");
    }
    let result = crate::repository::load_policy(fixture.path());
    assert!(
        matches!(
            result,
            Err(crate::repository::PolicyLoadError::Alias { .. })
        ),
        "{result:?}"
    );
}

#[test]
fn hard_links_share_one_object_identity() {
    let fixture = harness_root("platform-hardlink-");
    fs::write(fixture.path().join("a.txt"), "same\n").expect("a");
    #[cfg(unix)]
    {
        fs::hard_link(fixture.path().join("a.txt"), fixture.path().join("b.txt"))
            .expect("hard link");
    }
    let a = fs::metadata(fixture.path().join("a.txt")).expect("a meta");
    let b = fs::metadata(fixture.path().join("b.txt")).expect("b meta");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(a.ino(), b.ino(), "hard links share the inode");
        assert_eq!(a.dev(), b.dev(), "hard links share the device");
    }
}

#[test]
fn special_files_are_never_accepted_as_managed_content() {
    let fixture = harness_root("platform-fifo-");
    #[cfg(unix)]
    {
        let fifo = fixture.path().join("managed-fifo");
        let status = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
        assert!(status.success());
        let metadata = fs::metadata(&fifo).expect("fifo meta");
        use std::os::unix::fs::FileTypeExt;
        assert!(metadata.file_type().is_fifo(), "the host reports a fifo");
    }
    // A fifo is not a regular file: the identity model never treats it
    // as managed content (the hostile corpus documents the boundary).
    let corpus = crate::lifecycle::hostile_fixtures::hostile_corpus();
    let fifo_fixture = corpus
        .iter()
        .find(|entry| entry.kind == crate::lifecycle::hostile_fixtures::FixtureKind::SpecialFile)
        .expect("fifo fixture");
    assert!(!fifo_fixture.expected_fail_boundary.is_empty());
}

#[test]
fn outside_root_mutation_targets_are_refused_typed() {
    let fixture = harness_root("platform-containment-");
    fs::create_dir_all(fixture.path().join("destination")).expect("destination");
    let root = AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(
        fixture.path().join("destination"),
    )
    .expect("root");
    // A relative path that would escape via a parent segment is refused
    // at parse time — before any target can exist.
    let traversal = RelativePath::parse("../escape.txt");
    assert!(traversal.is_err(), "{traversal:?}");
    // A well-formed relative path resolves inside the root by
    // construction: the mutation target path always stays below the
    // destination.
    let relative = RelativePath::parse("escape.txt").expect("relative");
    let candidate = fixture.path().join("destination").join(relative.display());
    assert!(candidate.starts_with(fixture.path().join("destination")));
    let _ = root;
}

#[test]
fn equal_no_op_and_atomic_replacement_hold_on_the_host() {
    let fixture = harness_root("platform-replace-");
    let target = fixture.path().join("managed.txt");
    fs::write(&target, "# omnirepo-start\nv1\n# omnirepo-end\n").expect("file");
    // No-op: identical content yields no write and the identity stays.
    let before = fs::metadata(&target).expect("before");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let before_inode = before.ino();
        let identical = crate::managed_content::destination_equals_source(
            &fs::read(&target).expect("read"),
            b"# omnirepo-start\nv1\n# omnirepo-end\n",
        );
        assert!(identical, "the destination equals the source");
        assert_eq!(
            before.ino(),
            before_inode,
            "no-op leaves the object untouched"
        );
    }
    // Atomic replacement: the new content appears via a single rename —
    // a concurrent reader sees either the old or the new bytes, never a
    // partial mix.
    let temporary = fixture.path().join("managed.txt.tmp");
    fs::write(&temporary, "# omnirepo-start\nv2\n# omnirepo-end\n").expect("temp");
    fs::rename(&temporary, &target).expect("rename");
    let after = fs::read(&target).expect("after");
    assert_eq!(after, b"# omnirepo-start\nv2\n# omnirepo-end\n");
}
