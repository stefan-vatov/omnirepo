//! Focused proof for absent-section append content.

#![allow(dead_code, unused_imports)]

use super::{AppendError, build_absent_section_append};
use crate::managed_content::delimiters::lookup;
use crate::managed_content::partial_scan::{Topology, scan_partial};

fn yaml() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

#[test]
fn missing_and_empty_files_produce_exact_content() {
    // Missing file (empty content): canonical section only.
    let appended = build_absent_section_append("", yaml(), "a: 1").expect("append");
    assert_eq!(
        appended,
        Some("# omnirepo-start\na: 1\n# omnirepo-end\n".to_owned())
    );
    // Empty file + empty payload: no append needed.
    assert_eq!(
        build_absent_section_append("", yaml(), "").expect("append"),
        None
    );
}

#[test]
fn nonempty_no_final_newline_and_crlf_cases_are_exact() {
    // Nonempty without a final newline: one separator blank line.
    let appended = build_absent_section_append("a: 1", yaml(), "b: 2").expect("append");
    assert_eq!(
        appended,
        Some("a: 1\n\n# omnirepo-start\nb: 2\n# omnirepo-end\n".to_owned())
    );
    // CRLF content is preserved; the separator matches the file style.
    let appended = build_absent_section_append("a: 1\r\n", yaml(), "b: 2").expect("append");
    let content = appended.expect("append");
    assert!(content.starts_with("a: 1\r\n"), "{content:?}");
    assert!(
        content.contains("\r\n\r\n# omnirepo-start\n"),
        "{content:?}"
    );
}

#[test]
fn repeat_sync_finds_one_pair_and_is_a_no_op() {
    let appended = build_absent_section_append("a: 1", yaml(), "b: 2")
        .expect("append")
        .expect("append");
    // The appended content now carries exactly one pair.
    match scan_partial(&appended, yaml()) {
        Topology::ExactlyOne { .. } => {}
        other => panic!("expected exactly one pair, got {other}"),
    }
    // A repeated append is refused: no duplicate section is ever appended.
    let error = build_absent_section_append(&appended, yaml(), "b: 2").expect_err("repeat");
    assert!(
        matches!(error, AppendError::SectionAlreadyPresent { .. }),
        "{error}"
    );
}

#[test]
fn ambiguous_existing_markers_refuse_the_append() {
    let hostile = "# omnirepo-start\nno close\n";
    let error = build_absent_section_append(hostile, yaml(), "x").expect_err("ambiguous");
    assert!(
        matches!(error, AppendError::SectionAlreadyPresent { .. }),
        "{error}"
    );
}
