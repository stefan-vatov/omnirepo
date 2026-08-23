//! Focused proof for the authority-typed source declaration reader.

#![allow(dead_code, unused_imports)]

use super::read_source_declarations;
use crate::platform::{AuthorityRoot, ReadOnly, RelativePath, SourceSnapshotRoot};
use crate::source::{DECLARATION_VERSION, RevisionId, SourceId};
use std::{fs, path::Path};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}
fn relative(value: &str) -> RelativePath {
    RelativePath::parse(value).expect("relative path")
}

fn fixture() -> (
    tempfile::TempDir,
    AuthorityRoot<SourceSnapshotRoot, ReadOnly>,
) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("source-catalog-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = AuthorityRoot::<SourceSnapshotRoot, ReadOnly>::open(fixture.path())
        .expect("authority root");
    (fixture, root)
}

#[test]
fn declarations_read_through_the_snapshot_authority() {
    let (_fixture, root) = fixture();
    fs::write(
        root.display_path().as_path().join("declarations.txt"),
        format!("{DECLARATION_VERSION}\nsource=upstream path=apps/a.yaml mode=sync\n"),
    )
    .expect("declaration file");
    let declarations = read_source_declarations(
        &root,
        &source("upstream"),
        &revision("rev-abc"),
        &[relative("declarations.txt")],
    )
    .expect("read");
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].path, "apps/a.yaml");
    assert_eq!(
        declarations[0].fields,
        vec![("mode".to_owned(), "sync".to_owned())]
    );
}

#[test]
fn aliased_and_directory_targets_fail_closed() {
    let (_fixture, root) = fixture();
    // A symlink alias must be rejected by the no-follow resolution.
    let alias = root.display_path().as_path().join("alias.txt");
    std::os::unix::fs::symlink("declarations.txt", &alias).expect("symlink");
    fs::write(
        root.display_path().as_path().join("declarations.txt"),
        format!("{DECLARATION_VERSION}\n"),
    )
    .expect("declaration file");
    let error = read_source_declarations(
        &root,
        &source("upstream"),
        &revision("rev-abc"),
        &[relative("alias.txt")],
    )
    .expect_err("alias must fail closed");
    assert!(format!("{error}").contains("unreadable"), "{error}");

    // A directory target is not a regular file.
    fs::create_dir_all(root.display_path().as_path().join("dir.txt")).expect("dir");
    let error = read_source_declarations(
        &root,
        &source("upstream"),
        &revision("rev-abc"),
        &[relative("dir.txt")],
    )
    .expect_err("directory must fail closed");
    assert!(format!("{error}").contains("unreadable"), "{error}");
}

#[test]
fn absent_target_is_unreadable() {
    let (_fixture, root) = fixture();
    let error = read_source_declarations(
        &root,
        &source("upstream"),
        &revision("rev-abc"),
        &[relative("missing.txt")],
    )
    .expect_err("absent must fail closed");
    assert!(format!("{error}").contains("unreadable"), "{error}");
}
