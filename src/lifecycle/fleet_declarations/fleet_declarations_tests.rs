//! Focused proof for reading the pinned source declarations through the
//! typed source root.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_declarations::read_pinned_declarations;
use crate::source::{RevisionId, SourceId};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-declarations-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}

fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn source_repo(root: &Path) {
    fs::create_dir_all(root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Declarations"]);
    git(&["config", "user.email", "declarations@example.test"]);
}

fn declaration_line(source: &str, path: &str, fields: &[(&str, &str)]) -> String {
    let mut line = format!("source={source} path={path}");
    for (key, value) in fields {
        line.push_str(&format!(" {key}={value}"));
    }
    line
}

#[test]
fn the_canonical_declaration_file_reads_through_the_typed_root_in_order() {
    let fixture = fixture_base();
    let root = fixture.path().join("source-a");
    source_repo(&root);
    let content = format!(
        "omnirepo-declarations-v1\n{}\n{}\n",
        declaration_line("source-a", "apps/app.yaml", &[("mode", "sync")]),
        declaration_line("source-a", "apps/app2.yaml", &[("mode", "verify")])
    );
    fs::create_dir_all(root.join(".omnirepo")).expect("declaration dir");
    fs::write(root.join(".omnirepo/source.yaml"), content).expect("declaration file");
    let declarations =
        read_pinned_declarations(&source_id("source-a"), &revision("rev-1"), &root).expect("read");
    assert_eq!(declarations.len(), 2, "declared order preserved");
    assert_eq!(declarations[0].path, "apps/app.yaml");
    assert_eq!(
        declarations[0].fields,
        vec![("mode".to_owned(), "sync".to_owned())]
    );
    assert_eq!(declarations[1].path, "apps/app2.yaml");
    assert!(declarations[0].provenance.contains("source.yaml:1"));
}

#[test]
fn malformed_declarations_fail_typed_with_source_and_file_naming() {
    let fixture = fixture_base();
    let root = fixture.path().join("source-b");
    source_repo(&root);
    fs::create_dir_all(root.join(".omnirepo")).expect("declaration dir");
    fs::write(root.join(".omnirepo/source.yaml"), "not-a-declaration\n").expect("file");
    let error = read_pinned_declarations(&source_id("source-b"), &revision("rev-1"), &root)
        .expect_err("malformed");
    assert!(
        error.contains("source-b") && error.contains("source.yaml"),
        "{error}"
    );
}

#[test]
fn a_missing_declaration_file_fails_typed_not_silently_absent() {
    let fixture = fixture_base();
    let root = fixture.path().join("source-c");
    source_repo(&root);
    let error = read_pinned_declarations(&source_id("source-c"), &revision("rev-1"), &root)
        .expect_err("missing");
    assert!(error.contains("source-c"), "{error}");
}

#[test]
fn an_unsupported_version_fails_typed() {
    let fixture = fixture_base();
    let root = fixture.path().join("source-d");
    source_repo(&root);
    fs::create_dir_all(root.join(".omnirepo")).expect("declaration dir");
    fs::write(
        root.join(".omnirepo/source.yaml"),
        "omnirepo-declarations-v99\n",
    )
    .expect("file");
    let error = read_pinned_declarations(&source_id("source-d"), &revision("rev-1"), &root)
        .expect_err("version");
    assert!(error.contains("source-d"), "{error}");
}
