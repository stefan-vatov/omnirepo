//! Focused proof for exact representation preservation.

#![allow(dead_code, unused_imports)]

use super::{Representation, check_exact_representation, destination_equals_source};

#[test]
fn destination_managed_representation_equals_source_exactly() {
    // Byte-for-byte equality; no normalization and no semantic merge.
    let source = b"\xef\xbb\xbfmanaged: v1\n";
    assert!(destination_equals_source(source, source));
    assert!(!destination_equals_source(b"managed: v1\n", source));
    assert!(!destination_equals_source(
        b"managed: v1\r\n",
        b"managed: v1\n"
    ));
}

#[test]
fn unsupported_representations_fail_before_write() {
    let invalid_utf8 = [0xff, 0xfe, 0x41];
    assert_eq!(
        check_exact_representation(&invalid_utf8, true),
        Representation::Unsupported {
            reason: "the source bytes are not valid UTF-8 for a UTF-8 target".to_owned()
        }
    );
    // A binary target carries any bytes exactly.
    assert_eq!(
        check_exact_representation(&invalid_utf8, false),
        Representation::Exact
    );
}

#[test]
fn no_normalization_ever_occurs() {
    // Even a trailing newline difference is a difference: the check never
    // normalizes line endings or trims.
    assert!(!destination_equals_source(b"a\n", b"a"));
    assert!(!destination_equals_source(b"a", b"a\n"));
    assert!(!destination_equals_source(b"a\n", b"a\r\n"));
}
