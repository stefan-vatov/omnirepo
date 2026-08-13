//! Focused proof for section replacement content building.

#![allow(dead_code, unused_imports)]

use super::{SectionError, SectionReplacement, build_section_replacement, escape_payload};
use crate::managed_content::delimiters::lookup;
use crate::managed_content::partial_scan::{Bounds, scan_partial};

fn yaml() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

fn bounds_of(content: &str) -> Bounds {
    match scan_partial(content, yaml()) {
        crate::managed_content::partial_scan::Topology::ExactlyOne { bounds } => bounds,
        other => panic!("expected exactly one section: {other}"),
    }
}

#[test]
fn arbitrary_valid_outside_content_remains_identical() {
    let original = "a: 1\nb: 2\n# omnirepo-start\nold-body\n# omnirepo-end\nc: 3\n";
    let replacement = build_section_replacement(original, yaml(), bounds_of(original), "new-body")
        .expect("build");
    assert!(
        replacement.content.starts_with("a: 1\nb: 2\n"),
        "{}",
        replacement.content
    );
    assert!(
        replacement.content.ends_with("c: 3\n"),
        "{}",
        replacement.content
    );
    assert!(
        replacement
            .content
            .contains("# omnirepo-start\nnew-body\n# omnirepo-end\n")
    );
    assert!(replacement.changed);
}

#[test]
fn empty_and_adjacent_sections_work() {
    // An empty payload produces an empty body between the markers.
    let original = "# omnirepo-start\nold\n# omnirepo-end\n";
    let replacement =
        build_section_replacement(original, yaml(), bounds_of(original), "").expect("build");
    assert_eq!(
        replacement.content, "# omnirepo-start\n# omnirepo-end\n",
        "empty body"
    );
    assert!(replacement.changed);
    // Adjacent sections: two pairs back to back both survive.  The
    // scanner reports the multi-pair content as ambiguous; the builder is
    // driven with the first pair's explicit bounds and keeps the second
    // pair's lines outside, verbatim.
    let adjacent = "# omnirepo-start\na\n# omnirepo-end\n# omnirepo-start\nb\n# omnirepo-end\n";
    let replacement = build_section_replacement(
        adjacent,
        yaml(),
        Bounds {
            start_line: 1,
            end_line: 3,
        },
        "x",
    )
    .expect("build");
    assert!(
        replacement
            .content
            .contains("x\n# omnirepo-end\n# omnirepo-start\nb\n"),
        "{}",
        replacement.content
    );
}

#[test]
fn marker_like_payload_text_follows_escaping_rules() {
    let payload = "line1\n# omnirepo-end\nline2";
    let escaped = escape_payload(payload, yaml());
    assert!(escaped.contains("# omnirepo-escaped"), "{escaped}");
    // The escaped payload cannot close the section.
    let original = "# omnirepo-start\nold\n# omnirepo-end\n";
    let replacement =
        build_section_replacement(original, yaml(), bounds_of(original), &escaped).expect("build");
    assert!(
        replacement.content.contains("# omnirepo-escaped"),
        "{}",
        replacement.content
    );
}

#[test]
fn equal_bodies_produce_no_transaction_request() {
    let original = "# omnirepo-start\nbody\n# omnirepo-end\n";
    let replacement =
        build_section_replacement(original, yaml(), bounds_of(original), "body").expect("build");
    assert!(!replacement.changed, "equal bodies are a no-op");
    assert_eq!(replacement.content, original);
}

#[test]
fn invalid_bounds_fail_typed() {
    let original = "# omnirepo-start\nbody\n# omnirepo-end\n";
    let error = build_section_replacement(
        original,
        yaml(),
        Bounds {
            start_line: 99,
            end_line: 100,
        },
        "x",
    )
    .expect_err("invalid bounds");
    assert!(
        matches!(error, SectionError::BoundsOutside { .. }),
        "{error}"
    );
}
