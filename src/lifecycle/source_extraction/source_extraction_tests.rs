//! Focused proof for authority-typed payload extraction.

#![allow(dead_code, unused_imports)]

use super::extract_from_snapshot;
use crate::platform::{AuthorityRoot, ReadOnly, SourceSnapshotRoot};
use crate::source::{PayloadKind, content_identity};
use std::{fs, path::Path};

fn fixture() -> (
    tempfile::TempDir,
    AuthorityRoot<SourceSnapshotRoot, ReadOnly>,
) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("source-extraction-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = AuthorityRoot::<SourceSnapshotRoot, ReadOnly>::open(fixture.path())
        .expect("authority root");
    fs::create_dir_all(fixture.path().join("managed")).expect("managed dir");
    (fixture, root)
}

#[test]
fn whole_file_bytes_are_exact_through_the_authority() {
    let (_fixture, root) = fixture();
    let content = b"\xef\xbb\xbfalpha\nbeta\n".as_slice();
    fs::write(
        root.display_path().as_path().join("managed/app.txt"),
        content,
    )
    .expect("write");
    let payload =
        extract_from_snapshot(&root, "managed/app.txt", &PayloadKind::WholeFile).expect("extract");
    assert_eq!(payload.bytes, content);
    assert_eq!(payload.content_identity, content_identity(content));
}

#[test]
fn section_extraction_resolves_through_the_authority() {
    let (_fixture, root) = fixture();
    fs::write(
        root.display_path().as_path().join("managed/app.txt"),
        b"one\ntwo\nthree\n",
    )
    .expect("write");
    let payload = extract_from_snapshot(
        &root,
        "managed/app.txt",
        &PayloadKind::Section {
            start_line: 2,
            end_line: 2,
        },
    )
    .expect("extract");
    assert_eq!(payload.bytes, b"two\n");
    assert_eq!(payload.section, Some((2, 2)));
}

#[test]
fn escaping_aliased_and_missing_locators_fail_contextually() {
    let (_fixture, root) = fixture();
    assert!(extract_from_snapshot(&root, "../escape", &PayloadKind::WholeFile).is_err());
    assert!(extract_from_snapshot(&root, "/absolute", &PayloadKind::WholeFile).is_err());
    assert!(extract_from_snapshot(&root, "missing.txt", &PayloadKind::WholeFile).is_err());
    // A directory target is not a regular file.
    fs::create_dir_all(root.display_path().as_path().join("dir.txt")).expect("dir");
    assert!(extract_from_snapshot(&root, "dir.txt", &PayloadKind::WholeFile).is_err());
}
