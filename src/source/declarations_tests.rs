//! Focused proof for pinned-snapshot source declaration parsing.

#![allow(dead_code, unused_imports)]

use super::declarations::{
    DECLARATION_VERSION, DeclarationsError, SourceDeclaration, parse_declarations,
};
use crate::source::snapshot::{RevisionId, SourceId};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn declaration_line(source: &str, revision: &str, path: &str, fields: &[(&str, &str)]) -> String {
    let mut line = format!("source={source} revision={revision} path={path}");
    for (key, value) in fields {
        line.push_str(&format!(" {key}={value}"));
    }
    line
}

#[test]
fn records_parse_with_order_and_provenance() {
    let content = format!(
        "{DECLARATION_VERSION}\n{}\n{}\n",
        declaration_line("upstream", "rev-abc", "apps/app.yaml", &[("mode", "sync")]),
        declaration_line(
            "upstream",
            "rev-abc",
            "apps/app2.yaml",
            &[("mode", "verify")]
        )
    );
    let declarations = parse_declarations(
        &source("upstream"),
        &revision("rev-abc"),
        &[("declarations.txt", content)],
    )
    .expect("parse");
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0].path, "apps/app.yaml");
    assert_eq!(
        declarations[0].fields,
        vec![("mode".to_owned(), "sync".to_owned())]
    );
    assert_eq!(declarations[0].provenance, "declarations.txt:1");
    assert_eq!(declarations[1].path, "apps/app2.yaml");
    assert_eq!(declarations[1].provenance, "declarations.txt:2");
}

#[test]
fn order_spans_multiple_files() {
    let a = format!(
        "{DECLARATION_VERSION}\n{}\n",
        declaration_line("upstream", "rev-abc", "a", &[])
    );
    let b = format!(
        "{DECLARATION_VERSION}\n{}\n",
        declaration_line("upstream", "rev-abc", "b", &[])
    );
    let declarations = parse_declarations(
        &source("upstream"),
        &revision("rev-abc"),
        &[("a.txt", a), ("b.txt", b)],
    )
    .expect("parse");
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0].path, "a");
    assert_eq!(declarations[1].path, "b");
}

#[test]
fn unsupported_version_fails_typed() {
    let content = "omnirepo-declarations-v99\nsource=upstream revision=rev-abc path=x\n".to_owned();
    let error = parse_declarations(
        &source("upstream"),
        &revision("rev-abc"),
        &[("declarations.txt", content)],
    )
    .expect_err("unsupported version");
    assert!(
        matches!(error, DeclarationsError::UnsupportedVersion { .. }),
        "{error}"
    );
}

#[test]
fn malformed_record_identifies_source_file_and_entry() {
    let content = format!(
        "{DECLARATION_VERSION}\n{}\nnot-a-record\n{}\n",
        declaration_line("upstream", "rev-abc", "good", &[]),
        declaration_line("other", "rev-abc", "bad", &[])
    );
    let error = parse_declarations(
        &source("upstream"),
        &revision("rev-abc"),
        &[("declarations.txt", content)],
    )
    .expect_err("malformed");
    let text = format!("{error}");
    assert!(text.contains("upstream"), "{text}");
    assert!(text.contains("declarations.txt"), "{text}");
    assert!(text.contains("entry 2"), "{text}");
}

#[test]
fn empty_content_is_unsupported() {
    let error = parse_declarations(
        &source("upstream"),
        &revision("rev-abc"),
        &[("missing.txt", String::new())],
    )
    .expect_err("empty content");
    assert!(
        matches!(error, DeclarationsError::UnsupportedVersion { .. }),
        "{error}"
    );
}
