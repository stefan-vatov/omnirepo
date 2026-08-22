//! Focused proof for exact payload extraction.

#![allow(dead_code, unused_imports)]

use super::extraction::{
    ExtractedPayload, ExtractionError, PayloadKind, content_identity, extract_payload,
    validate_locator,
};

#[test]
fn whole_file_preserves_exact_bytes() {
    let content = b"\xef\xbb\xbfline1\nline2\nline3\n".as_slice();
    let extracted =
        extract_payload("dir/file.txt", content, &PayloadKind::WholeFile).expect("whole file");
    assert_eq!(extracted.bytes, content, "bytes must be exact");
    assert_eq!(extracted.section, None);
    assert_eq!(extracted.content_identity, content_identity(content));
    // A BOM is content, not structure: the identity covers it.
    assert_ne!(
        extracted.content_identity,
        content_identity(b"line1\nline2\nline3\n")
    );
}

#[test]
fn section_extraction_preserves_decided_lines_and_identity() {
    let content = b"a\nbb\nccc\ndddd\n";
    let section = extract_payload(
        "dir/file.txt",
        content,
        &PayloadKind::Section {
            start_line: 2,
            end_line: 3,
        },
    )
    .expect("section");
    assert_eq!(section.bytes, b"bb\nccc\n");
    assert_eq!(section.section, Some((2, 3)));
    assert_eq!(section.content_identity, content_identity(b"bb\nccc\n"));
}

#[test]
fn escaping_and_ambiguous_locators_fail_contextually() {
    assert!(matches!(
        validate_locator("../escape"),
        Err(ExtractionError::Escaping { .. })
    ));
    assert!(matches!(
        validate_locator("/absolute"),
        Err(ExtractionError::Escaping { .. })
    ));
    assert!(matches!(
        validate_locator("a//b"),
        Err(ExtractionError::Ambiguous { .. })
    ));
    assert!(matches!(
        validate_locator(""),
        Err(ExtractionError::Ambiguous { .. })
    ));
    assert!(matches!(
        validate_locator("a\0b"),
        Err(ExtractionError::Ambiguous { .. })
    ));
}

#[test]
fn invalid_sections_fail_typed() {
    let content = b"a\nb\nc\n";
    assert!(matches!(
        extract_payload(
            "f",
            content,
            &PayloadKind::Section {
                start_line: 0,
                end_line: 1
            }
        ),
        Err(ExtractionError::Section { .. })
    ));
    assert!(matches!(
        extract_payload(
            "f",
            content,
            &PayloadKind::Section {
                start_line: 4,
                end_line: 5
            }
        ),
        Err(ExtractionError::Section { .. })
    ));
    assert!(matches!(
        extract_payload(
            "f",
            content,
            &PayloadKind::Section {
                start_line: 3,
                end_line: 2
            }
        ),
        Err(ExtractionError::Section { .. })
    ));
}

#[test]
fn empty_content_still_extracts_an_identity() {
    let extracted = extract_payload("f", b"", &PayloadKind::WholeFile).expect("empty");
    assert!(extracted.bytes.is_empty());
    assert!(!extracted.content_identity.is_empty());
}
