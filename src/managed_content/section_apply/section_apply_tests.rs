//! Focused proof for grouped section application.

#![allow(dead_code, unused_imports)]

use super::{ApplyError, SectionWrite, apply_sections};
use crate::configuration::SectionId;
use crate::managed_content::delimiters::lookup;

fn yaml() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

fn markdown() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("markdown").expect("markdown")
}

fn write(id: &str, payload: &str) -> SectionWrite {
    SectionWrite {
        id: SectionId::new(id).expect("valid id"),
        payload: payload.as_bytes().to_vec(),
    }
}

#[test]
fn absent_sections_append_in_write_order_with_one_blank_separator() {
    let applied = apply_sections(
        b"# Local title\n",
        markdown(),
        &[write("alpha", "a-rules\n"), write("beta", "b-rules\n")],
    )
    .expect("apply");
    assert_eq!(
        String::from_utf8(applied.content).expect("utf8"),
        "# Local title\n\n<!-- omnirepo:start alpha -->\na-rules\n<!-- omnirepo:end alpha -->\n\n<!-- omnirepo:start beta -->\nb-rules\n<!-- omnirepo:end beta -->\n"
    );
    assert!(applied.changed);
    assert_eq!(applied.sections.len(), 2);
    assert!(applied.sections.iter().all(|section| !section.existed));
}

#[test]
fn empty_file_and_no_final_newline_cases_are_exact() {
    // Empty file: the first block starts at byte zero, no separator.
    let applied = apply_sections(b"", yaml(), &[write("a", "a: 1\n")]).expect("apply");
    assert_eq!(
        applied.content,
        b"# omnirepo:start a\na: 1\n# omnirepo:end a\n"
    );
    // No final newline: one terminator, then the blank separator line.
    let applied = apply_sections(b"local: 1", yaml(), &[write("a", "a: 1\n")]).expect("apply");
    assert_eq!(
        String::from_utf8(applied.content).expect("utf8"),
        "local: 1\n\n# omnirepo:start a\na: 1\n# omnirepo:end a\n"
    );
    // An empty payload still writes the marker pair.
    let applied = apply_sections(b"", yaml(), &[write("a", "")]).expect("apply");
    assert_eq!(applied.content, b"# omnirepo:start a\n# omnirepo:end a\n");
}

#[test]
fn existing_sections_replace_in_place_and_preserve_everything_else() {
    let original = "top: 1\n# omnirepo:start alpha\nold\n# omnirepo:end alpha\nmid: 2\n# omnirepo:start beta\nkeep\n# omnirepo:end beta\ntail: 3\n";
    let applied =
        apply_sections(original.as_bytes(), yaml(), &[write("alpha", "new\n")]).expect("apply");
    assert_eq!(
        String::from_utf8(applied.content).expect("utf8"),
        "top: 1\n# omnirepo:start alpha\nnew\n# omnirepo:end alpha\nmid: 2\n# omnirepo:start beta\nkeep\n# omnirepo:end beta\ntail: 3\n"
    );
    // The unwritten section beta is destination content this run: untouched.
    assert_eq!(applied.sections.len(), 1);
    assert!(applied.sections[0].existed);
    assert!(applied.sections[0].changed);
}

#[test]
fn equal_bodies_are_a_true_no_op() {
    let original = "# omnirepo:start a\nmanaged: true\n# omnirepo:end a\n";
    let applied = apply_sections(
        original.as_bytes(),
        yaml(),
        &[write("a", "managed: true\n")],
    )
    .expect("apply");
    assert!(!applied.changed);
    assert_eq!(applied.content, original.as_bytes());
    assert!(!applied.sections[0].changed);
    // Convergence: applying twice yields byte-identical content.
    let once = apply_sections(b"x: 1\n", yaml(), &[write("a", "managed: true\n")]).expect("one");
    let twice =
        apply_sections(&once.content, yaml(), &[write("a", "managed: true\n")]).expect("two");
    assert_eq!(once.content, twice.content);
    assert!(!twice.changed);
}

#[test]
fn crlf_style_is_detected_for_inserted_lines_and_payload_stays_exact() {
    let applied = apply_sections(b"local: 1\r\n", yaml(), &[write("a", "a: 1\n")]).expect("apply");
    let content = String::from_utf8(applied.content).expect("utf8");
    assert!(
        content.starts_with("local: 1\r\n\r\n# omnirepo:start a\r\n"),
        "{content:?}"
    );
    // Payload bytes are authoritative: the LF payload is not transcoded.
    assert!(content.contains("a: 1\n"), "{content:?}");
}

#[test]
fn marker_like_payload_and_hostile_topologies_fail_without_effect() {
    // Payload-like markers are invalid, never escaped.
    let error = apply_sections(b"", yaml(), &[write("a", "body\n# omnirepo:end a\n")])
        .expect_err("payload marker");
    assert!(matches!(error, ApplyError::PayloadMarker { .. }), "{error}");
    // A duplicate write in one group is a caller error.
    let error = apply_sections(b"", yaml(), &[write("a", "x\n"), write("a", "y\n")])
        .expect_err("duplicate");
    assert!(
        matches!(error, ApplyError::DuplicateWrite { .. }),
        "{error}"
    );
    // An ambiguous destination fails before any composition.
    let error = apply_sections(
        b"# omnirepo:start a\nno close\n",
        yaml(),
        &[write("a", "x\n")],
    )
    .expect_err("topology");
    assert!(matches!(error, ApplyError::Topology { .. }), "{error}");
}

#[test]
fn payload_crlf_never_flips_the_file_newline_style() {
    // EOL style is detected from the file's first line terminator, so a
    // CRLF-bearing authoritative payload cannot flip marker lines on the
    // next run: the second apply is a byte-exact no-op.
    let payload = "win: 1\r\nmore: 2\r\n";
    let first = apply_sections(b"local: 1\n", yaml(), &[write("a", payload)]).expect("first");
    let content = String::from_utf8(first.content.clone()).expect("utf8");
    assert!(
        content.contains("\n# omnirepo:start a\n"),
        "markers keep the file's LF style: {content:?}"
    );
    let second = apply_sections(&first.content, yaml(), &[write("a", payload)]).expect("second");
    assert!(!second.changed, "the second run is a true no-op");
    assert_eq!(second.content, first.content);
}

#[test]
fn prose_mentioning_markers_is_lawful_payload_and_content() {
    // Documentation about omnirepo markers must survive as payload and
    // as destination content: only marker-shaped lines are invalid.
    let payload = "Sections are delimited by omnirepo:start markers.\n";
    let applied = apply_sections(b"", markdown(), &[write("doc", payload)]).expect("apply");
    let second =
        apply_sections(&applied.content, markdown(), &[write("doc", payload)]).expect("second");
    assert!(!second.changed);
}

#[test]
fn legacy_unnamed_markers_fail_typed_instead_of_duplicating() {
    // A destination still carrying the previous release's unnamed
    // markers is refused, never silently appended beside.
    let legacy = "# omnirepo-start\nstale\n# omnirepo-end\n";
    let error = apply_sections(legacy.as_bytes(), yaml(), &[write("a", "new\n")])
        .expect_err("legacy markers");
    assert!(
        error.to_string().contains("legacy unnamed marker"),
        "{error}"
    );
}
