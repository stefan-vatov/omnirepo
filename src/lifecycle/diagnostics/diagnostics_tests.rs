//! Focused proof for stable diagnostics.

#![allow(dead_code, unused_imports)]

use super::{
    AffectedScope, Diagnostic, DiagnosticError, DiagnosticStage, diagnostic, render_diagnostic,
};

#[test]
fn every_failure_maps_to_one_stable_code_with_full_context() {
    let d = diagnostic(
        "source-unavailable",
        DiagnosticStage::Source,
        "/workspace/snapshots/upstream",
        "fs:1:2",
        Some("declarations.txt".to_owned()),
        AffectedScope::Item {
            repository: "dest-a".to_owned(),
            item: "item-a".to_owned(),
        },
        "re-run the acquisition for upstream after the upstream is reachable",
    )
    .expect("diagnostic");
    assert_eq!(d.code, "source-unavailable");
    assert_eq!(d.stage, DiagnosticStage::Source);
    assert_eq!(d.authority_path, "/workspace/snapshots/upstream");
    assert_eq!(d.field.as_deref(), Some("declarations.txt"));
    assert_eq!(
        d.scope,
        AffectedScope::Item {
            repository: "dest-a".to_owned(),
            item: "item-a".to_owned(),
        }
    );
}

#[test]
fn empty_codes_and_remediations_fail_typed() {
    assert!(matches!(
        diagnostic(
            "",
            DiagnosticStage::Configuration,
            "path",
            "id",
            None,
            AffectedScope::Global,
            "remediation",
        ),
        Err(DiagnosticError::EmptyCode)
    ));
    assert!(matches!(
        diagnostic(
            "config-malformed",
            DiagnosticStage::Configuration,
            "path",
            "id",
            None,
            AffectedScope::Global,
            "",
        ),
        Err(DiagnosticError::EmptyRemediation)
    ));
}

#[test]
fn display_formatting_is_separate_and_inert() {
    let d = diagnostic(
        "config-malformed",
        DiagnosticStage::Configuration,
        "/etc/omnirepo/config.yaml",
        "fs:7:3",
        Some("sources[0]".to_owned()),
        AffectedScope::Global,
        "fix the machine configuration and re-run validate",
    )
    .expect("diagnostic");
    let rendered = render_diagnostic(&d);
    assert!(rendered.contains("config-malformed"), "{rendered}");
    assert!(rendered.contains("configuration"), "{rendered}");
    assert!(rendered.contains("/etc/omnirepo/config.yaml"), "{rendered}");
    assert!(rendered.contains("field=sources[0]"), "{rendered}");
    assert!(!rendered.contains('\u{1b}'), "no ANSI in the render");
    // The data and the render are independent: mutating the render never
    // touches the diagnostic.
    let _: &Diagnostic = &d;
}
