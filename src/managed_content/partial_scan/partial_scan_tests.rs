//! Focused proof for named multi-section scanning.

#![allow(dead_code, unused_imports)]

use super::{Bounds, ScanOutcome, scan_sections};
use crate::configuration::SectionId;
use crate::managed_content::delimiters::lookup;

fn yaml() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

fn invalid(content: &str) -> String {
    match scan_sections(content.as_bytes(), yaml()) {
        ScanOutcome::Invalid { reason } => reason,
        other => panic!("expected invalid topology for {content:?}, got {other}"),
    }
}

#[test]
fn empty_one_and_many_sections_scan_in_file_order() {
    match scan_sections(b"plain: file\n", yaml()) {
        ScanOutcome::Sections(sections) => assert!(sections.is_empty()),
        other => panic!("expected no sections: {other}"),
    }
    let two = "top: 1\n# omnirepo:start alpha\na: 1\n# omnirepo:end alpha\nmid: 2\n# omnirepo:start beta\nb: 2\n# omnirepo:end beta\n";
    match scan_sections(two.as_bytes(), yaml()) {
        ScanOutcome::Sections(sections) => {
            assert_eq!(sections.len(), 2);
            assert_eq!(sections[0].id.as_str(), "alpha");
            assert_eq!(
                sections[0].bounds,
                Bounds {
                    start_line: 2,
                    end_line: 4
                }
            );
            assert_eq!(sections[1].id.as_str(), "beta");
            assert_eq!(
                sections[1].bounds,
                Bounds {
                    start_line: 6,
                    end_line: 8
                }
            );
        }
        other => panic!("expected two sections: {other}"),
    }
}

#[test]
fn every_ambiguous_topology_is_invalid_with_a_named_reason() {
    // Unpaired open.
    assert!(invalid("# omnirepo:start a\nno close\n").contains("unclosed"));
    // Close without open.
    assert!(invalid("# omnirepo:end a\n").contains("without an open marker"));
    // Reversed: the close precedes the open.
    assert!(invalid("# omnirepo:end a\n# omnirepo:start a\n").contains("without an open marker"));
    // Nested.
    assert!(
        invalid("# omnirepo:start a\n# omnirepo:start b\n# omnirepo:end b\n# omnirepo:end a\n")
            .contains("inside the open section")
    );
    // Interleaved / mismatched close.
    assert!(invalid("# omnirepo:start a\n# omnirepo:end b\n").contains("while section a is open"));
    // Duplicate section id.
    assert!(
        invalid("# omnirepo:start a\n# omnirepo:end a\n# omnirepo:start a\n# omnirepo:end a\n")
            .contains("duplicate section id")
    );
    // Unnamed (legacy) markers are marker-like, not markers.
    assert!(invalid("# omnirepo:start\nx\n# omnirepo:end\n").contains("resembles a marker"));
}

#[test]
fn scanning_never_mutates_and_ignores_terminator_style() {
    let crlf = "# omnirepo:start a\r\nbody\r\n# omnirepo:end a\r\n";
    let snapshot = crlf.to_owned();
    match scan_sections(crlf.as_bytes(), yaml()) {
        ScanOutcome::Sections(sections) => {
            assert_eq!(sections.len(), 1);
            assert_eq!(sections[0].id.as_str(), "a");
        }
        other => panic!("expected one section: {other}"),
    }
    assert_eq!(crlf, snapshot, "scanning never mutates the content");
}
