//! Focused proof for authenticated control framing and output sanitation.

#![allow(dead_code, unused_imports)]

use super::{ControlFrame, FrameError, frame, parse_frame, sanitize_output};

#[test]
fn control_frames_round_trip_and_reject_hostiles() {
    let framed = frame("token-abc", "verify --check").expect("frame");
    let parsed = parse_frame(&framed).expect("parse");
    assert_eq!(
        parsed,
        ControlFrame {
            token: "token-abc".to_owned(),
            payload: "verify --check".to_owned()
        }
    );
    // Unknown version, missing fields, wrong length, and oversized inputs
    // fail closed.
    assert!(matches!(
        parse_frame("omnirepo-control-v2 token=a payload=1:x"),
        Err(FrameError::UnknownVersion { .. })
    ));
    assert!(matches!(
        parse_frame("omnirepo-control-v1 payload=1:x"),
        Err(FrameError::Malformed { .. })
    ));
    assert!(matches!(
        parse_frame("omnirepo-control-v1 token=a payload=9:x"),
        Err(FrameError::Oversized { .. })
    ));
    assert!(frame("token with space", "x").is_err());
    assert!(frame("token", "multi\nline").is_err());
    assert!(frame("token", &"x".repeat(super::MAX_FRAME_PAYLOAD_BYTES + 1)).is_err());
}

#[test]
fn untrusted_output_is_sanitized_to_inert_text() {
    let hostile = "\u{1b}[31mred\u{1b}[0m plain\n\tindented\u{7}bell\u{1b}]0;title\u{7}";
    let sanitized = sanitize_output(hostile);
    assert!(!sanitized.contains('\u{1b}'), "{sanitized:?}");
    assert!(!sanitized.contains('\u{7}'), "{sanitized:?}");
    assert!(sanitized.contains("red"), "{sanitized:?}");
    assert!(sanitized.contains("plain"), "{sanitized:?}");
    assert!(sanitized.contains('\n'), "newlines survive");
    assert!(sanitized.contains('\t'), "tabs survive");
}
