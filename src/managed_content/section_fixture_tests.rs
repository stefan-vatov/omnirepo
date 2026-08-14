//! Property, ambiguity, idempotence, and boundary fixtures for the
//! partial-section machinery.

#![allow(dead_code, unused_imports)]

use crate::managed_content::delimiters::{DelimiterError, lookup, lookup_by_extension};
use crate::managed_content::partial_scan::{Topology, scan_partial};
use crate::managed_content::section_append::build_absent_section_append;
use crate::managed_content::section_builder::build_section_replacement;

fn yaml() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

#[test]
fn every_malformed_case_preserves_full_file_identity() {
    let malformed = [
        "# omnirepo-start\nno close\n",
        "# omnirepo-end\nno open\n",
        "# omnirepo-end\n# omnirepo-start\n",
        "# omnirepo-start\nx\n# omnirepo-end\n# omnirepo-start\ny\n# omnirepo-end\n",
    ];
    for content in malformed {
        let snapshot = content.to_owned();
        let topology = scan_partial(content, yaml());
        assert!(
            matches!(topology, Topology::Ambiguous { .. }),
            "{content:?}"
        );
        assert_eq!(content, snapshot, "scanning never mutates the file");
    }
}

#[test]
fn existing_and_absent_cases_converge_in_one_run_and_no_op_in_two() {
    // Absent: one append converges; the second is refused (no duplicate).
    let first = build_absent_section_append("a: 1", yaml(), "managed: true")
        .expect("append")
        .expect("append");
    match scan_partial(&first, yaml()) {
        Topology::ExactlyOne { .. } => {}
        other => panic!("one pair after the first run: {other}"),
    }
    assert!(
        build_absent_section_append(&first, yaml(), "managed: true").is_err(),
        "a second append is refused"
    );
    // Existing: an equal body is a true no-op on the second run.
    let existing = "# omnirepo-start\nmanaged: true\n# omnirepo-end\n";
    let rebuilt = build_section_replacement(existing, yaml(), bounds_of(existing), "managed: true")
        .expect("rebuild");
    assert!(!rebuilt.changed);
    assert_eq!(rebuilt.content, existing);
}

#[test]
fn outside_content_is_exact() {
    let original = "a: 1\nb: 2\n# omnirepo-start\nold\n# omnirepo-end\nc: 3\n";
    let rebuilt =
        build_section_replacement(original, yaml(), bounds_of(original), "new").expect("rebuild");
    assert!(
        rebuilt.content.starts_with("a: 1\nb: 2\n"),
        "{}",
        rebuilt.content
    );
    assert!(rebuilt.content.ends_with("c: 3\n"), "{}", rebuilt.content);
    // Byte-exact outside lines: nothing normalized.
    let crlf = "a: 1\r\n# omnirepo-start\nold\n# omnirepo-end\n";
    let rebuilt = build_section_replacement(crlf, yaml(), bounds_of(crlf), "x").expect("rebuild");
    assert!(
        rebuilt.content.starts_with("a: 1\r\n"),
        "{:?}",
        rebuilt.content
    );
}

#[test]
fn unknown_format_and_extension_matching_follow_policy() {
    // Unknown formats fail closed with the typed reason.
    assert!(matches!(
        lookup("makefile"),
        Err(DelimiterError::UnknownFormat { .. })
    ));
    // Extensionless and unknown extensions fail closed.
    assert!(matches!(
        lookup_by_extension("apps/Dockerfile"),
        Err(DelimiterError::UnknownExtension { .. })
    ));
    assert!(matches!(
        lookup_by_extension("apps/app.xyz"),
        Err(DelimiterError::UnknownExtension { .. })
    ));
    // Known extensions map exactly; case is folded by rule.
    assert_eq!(lookup_by_extension("a.yaml").expect("yaml").format, "yaml");
    assert_eq!(lookup_by_extension("a.YML").expect("yaml").format, "yaml");
    assert_eq!(lookup_by_extension("a.md").expect("md").format, "markdown");
    assert_eq!(
        lookup_by_extension("a.ts").expect("ts").format,
        "typescript"
    );
}

fn bounds_of(content: &str) -> crate::managed_content::partial_scan::Bounds {
    match scan_partial(content, yaml()) {
        Topology::ExactlyOne { bounds } => bounds,
        other => panic!("expected exactly one section: {other}"),
    }
}
