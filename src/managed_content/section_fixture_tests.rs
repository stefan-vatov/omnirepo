//! Property, ambiguity, idempotence, and boundary fixtures for the
//! named partial-section machinery.

#![allow(dead_code, unused_imports)]

use crate::configuration::SectionId;
use crate::managed_content::delimiters::{DelimiterError, lookup, lookup_by_extension};
use crate::managed_content::partial_scan::{ScanOutcome, scan_sections};
use crate::managed_content::section_apply::{SectionWrite, apply_sections};

fn yaml() -> &'static crate::managed_content::delimiters::DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

fn write(id: &str, payload: &str) -> SectionWrite {
    SectionWrite {
        id: SectionId::new(id).expect("valid id"),
        payload: payload.as_bytes().to_vec(),
    }
}

#[test]
fn every_malformed_case_preserves_full_file_identity() {
    let malformed = [
        "# omnirepo:start a\nno close\n",
        "# omnirepo:end a\nno open\n",
        "# omnirepo:end a\n# omnirepo:start a\n",
        "# omnirepo:start a\nx\n# omnirepo:end b\n",
        "# omnirepo:start a\nx\n# omnirepo:end a\n# omnirepo:start a\ny\n# omnirepo:end a\n",
        "# omnirepo:start\nunnamed legacy markers\n# omnirepo:end\n",
    ];
    for content in malformed {
        let snapshot = content.to_owned();
        let outcome = scan_sections(content.as_bytes(), yaml());
        assert!(
            matches!(outcome, ScanOutcome::Invalid { .. }),
            "{content:?}"
        );
        assert!(
            apply_sections(content.as_bytes(), yaml(), &[write("a", "x\n")]).is_err(),
            "{content:?}"
        );
        assert_eq!(content, snapshot, "scanning never mutates the file");
    }
}

#[test]
fn two_named_sections_share_one_file_and_converge() {
    // Two distinct IDs are valid in one destination file; a run that
    // writes both converges in one pass and no-ops on the second.
    let first = apply_sections(
        b"local: 1\n",
        yaml(),
        &[write("alpha", "a: 1\n"), write("beta", "b: 2\n")],
    )
    .expect("first run");
    match scan_sections(&first.content, yaml()) {
        ScanOutcome::Sections(sections) => {
            assert_eq!(sections.len(), 2, "both sections landed");
        }
        other => panic!("expected two sections: {other}"),
    }
    let second = apply_sections(
        &first.content,
        yaml(),
        &[write("alpha", "a: 1\n"), write("beta", "b: 2\n")],
    )
    .expect("second run");
    assert!(!second.changed, "the second run is a true no-op");
    assert_eq!(second.content, first.content);
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
    assert_eq!(
        lookup_by_extension("a.ts").expect("ts").format,
        "typescript"
    );
}
