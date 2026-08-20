//! Focused proof for the named delimiter registry and marker parsing.

#![allow(dead_code, unused_imports)]

use super::{DelimiterError, LineClass, lookup, lookup_by_extension};
use crate::configuration::SectionId;

fn id(value: &str) -> SectionId {
    SectionId::new(value).expect("valid id")
}

#[test]
fn markers_round_trip_per_format() {
    let yaml = lookup("yaml").expect("yaml");
    assert_eq!(yaml.open_marker(&id("rules")), "# omnirepo:start rules");
    assert_eq!(yaml.close_marker(&id("rules")), "# omnirepo:end rules");
    let markdown = lookup("markdown").expect("markdown");
    assert_eq!(
        markdown.open_marker(&id("rust-rules")),
        "<!-- omnirepo:start rust-rules -->"
    );
    assert_eq!(
        markdown.close_marker(&id("rust-rules")),
        "<!-- omnirepo:end rust-rules -->"
    );
    let sql = lookup("sql").expect("sql");
    assert_eq!(sql.open_marker(&id("s.1")), "-- omnirepo:start s.1");
    let ini = lookup("ini").expect("ini");
    assert_eq!(ini.close_marker(&id("s_2")), "; omnirepo:end s_2");
}

#[test]
fn classification_is_exact_and_named() {
    let yaml = lookup("yaml").expect("yaml");
    assert_eq!(
        yaml.classify_line(b"# omnirepo:start rules"),
        LineClass::Open(id("rules"))
    );
    assert_eq!(
        yaml.classify_line(b"# omnirepo:end rules"),
        LineClass::Close(id("rules"))
    );
    assert_eq!(yaml.classify_line(b"ordinary: line"), LineClass::Content);
    // Marker-shaped lines that are not exact named markers are
    // marker-like: invalid, never content and never a marker.
    for hostile in [
        b"# omnirepo:start".as_slice(),
        b"  # omnirepo:start rules",
        b"# omnirepo:start Rules",
        b"#omnirepo:start rules",
        b"# omnirepo:start rules trailing",
        b"omnirepo:end rules",
    ] {
        assert!(
            matches!(yaml.classify_line(hostile), LineClass::MarkerLike { .. }),
            "{:?}",
            String::from_utf8_lossy(hostile)
        );
    }
    // Prose that mentions a keyword mid-line is ordinary content: the
    // exact-marker rule governs marker-shaped lines, not documentation
    // about them.
    for prose in [
        b"say omnirepo:end somewhere".as_slice(),
        b"add `# omnirepo:start rules` to the file",
        b"sections are delimited by omnirepo:start markers",
    ] {
        assert_eq!(
            yaml.classify_line(prose),
            LineClass::Content,
            "{:?}",
            String::from_utf8_lossy(prose)
        );
    }
    // Legacy unnamed markers are refused, never content: silently
    // appending beside a stale legacy block would duplicate content.
    for legacy in [
        b"# omnirepo-start".as_slice(),
        b"# omnirepo-end",
        b"omnirepo-start",
    ] {
        assert!(
            matches!(yaml.classify_line(legacy), LineClass::MarkerLike { .. }),
            "{:?}",
            String::from_utf8_lossy(legacy)
        );
    }
    // A prose mention of the legacy keyword stays content.
    assert_eq!(
        yaml.classify_line(b"the old omnirepo-start form is gone"),
        LineClass::Content
    );
    // The markdown suffix must close the comment exactly.
    let markdown = lookup("markdown").expect("markdown");
    assert_eq!(
        markdown.classify_line(b"<!-- omnirepo:start rust-rules -->"),
        LineClass::Open(id("rust-rules"))
    );
    assert!(matches!(
        markdown.classify_line(b"<!-- omnirepo:start rust-rules"),
        LineClass::MarkerLike { .. }
    ));
    assert!(matches!(
        markdown.classify_line(b"<!-- omnirepo-start -->"),
        LineClass::MarkerLike { .. }
    ));
}

#[test]
fn unknown_format_and_extension_matching_follow_policy() {
    assert!(matches!(
        lookup("makefile"),
        Err(DelimiterError::UnknownFormat { .. })
    ));
    assert!(matches!(
        lookup_by_extension("apps/Dockerfile"),
        Err(DelimiterError::UnknownExtension { .. })
    ));
    assert!(matches!(
        lookup_by_extension("apps/app.xyz"),
        Err(DelimiterError::UnknownExtension { .. })
    ));
    assert_eq!(lookup_by_extension("a.yaml").expect("yaml").format, "yaml");
    assert_eq!(lookup_by_extension("a.YML").expect("yaml").format, "yaml");
    assert_eq!(lookup_by_extension("a.md").expect("md").format, "markdown");
    assert_eq!(lookup_by_extension("a.rb").expect("rb").format, "ruby");
    assert_eq!(lookup_by_extension("a.sql").expect("sql").format, "sql");
    assert_eq!(lookup_by_extension("a.ini").expect("ini").format, "ini");
}
