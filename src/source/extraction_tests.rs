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
    assert_eq!(extracted.content_identity, content_identity(content));
    // A BOM is content, not structure: the identity covers it.
    assert_ne!(
        extracted.content_identity,
        content_identity(b"line1\nline2\nline3\n")
    );
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
fn empty_content_still_extracts_an_identity() {
    let extracted = extract_payload("f", b"", &PayloadKind::WholeFile).expect("empty");
    assert!(extracted.bytes.is_empty());
    assert!(!extracted.content_identity.is_empty());
}
