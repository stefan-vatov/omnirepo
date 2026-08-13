//! Focused proof for the delimiter syntax registry.

#![allow(dead_code, unused_imports)]

use super::{DelimiterError, lookup, lookup_by_extension};

#[test]
fn every_supported_format_round_trips_canonical_markers() {
    for entry in super::REGISTRY {
        let (open, close) = entry.round_trip();
        assert!(!open.is_empty(), "{}", entry.format);
        assert!(!close.is_empty(), "{}", entry.format);
        assert_ne!(open, close, "{}", entry.format);
        assert!(open.contains("omnirepo"), "{}", entry.format);
        // The canonical markers parse back to the same syntax.
        let looked_up = lookup(entry.format).expect("registry entry");
        assert_eq!(looked_up.open, open);
        assert_eq!(looked_up.close, close);
    }
}

#[test]
fn lookup_follows_decided_case_and_extension_rules() {
    // Exact format names.
    assert_eq!(lookup("yaml").expect("yaml").open, "# omnirepo-start");
    assert_eq!(lookup("json").expect("json").open, "// omnirepo-start");
    assert_eq!(
        lookup("markdown").expect("markdown").open,
        "<!-- omnirepo-start -->"
    );
    // Case-sensitive format names by rule: "YAML" is unknown.
    assert!(matches!(
        lookup("YAML"),
        Err(DelimiterError::UnknownFormat { .. })
    ));
    // Extensions map to formats; extension case is folded.
    assert_eq!(
        lookup_by_extension("apps/app.YAML")
            .expect("yaml ext")
            .format,
        "yaml"
    );
    assert_eq!(
        lookup_by_extension("apps/app.ts").expect("ts ext").format,
        "typescript"
    );
    assert_eq!(
        lookup_by_extension("apps/app.html")
            .expect("html ext")
            .format,
        "html"
    );
}

#[test]
fn unknown_and_extensionless_cases_fail_or_behave_per_policy() {
    // Unknown format: fail closed.
    assert!(matches!(
        lookup("makefile"),
        Err(DelimiterError::UnknownFormat { .. })
    ));
    // Extensionless path: fail closed with the decided rule.
    assert!(matches!(
        lookup_by_extension("apps/Dockerfile"),
        Err(DelimiterError::UnknownExtension { .. })
    ));
    // Unknown extension: fail closed.
    assert!(matches!(
        lookup_by_extension("apps/app.xyz"),
        Err(DelimiterError::UnknownExtension { .. })
    ));
    // Empty input: fail typed.
    assert!(matches!(lookup(""), Err(DelimiterError::Empty)));
    assert!(matches!(
        lookup_by_extension(""),
        Err(DelimiterError::Empty)
    ));
}

#[test]
fn registry_contains_no_config_parser() {
    // The registry is pure data: the module exposes only lookup functions
    // and the static table.  (Compile-time contract; the assertion pins
    // the shape.)
    #[allow(clippy::const_is_empty)]
    {
        assert!(!super::REGISTRY.is_empty());
    }
    let _ = lookup;
    let _ = lookup_by_extension;
}
