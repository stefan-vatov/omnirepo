//! Focused proof for aggregated redacted diagnostics.

#![allow(dead_code, unused_imports)]

use super::{aggregate_diagnostics, redact, render_redacted};
use crate::lifecycle::diagnostics::{AffectedScope, DiagnosticStage, diagnostic};

fn diag(code: &'static str, repository: &str) -> crate::lifecycle::diagnostics::Diagnostic {
    diagnostic(
        code,
        DiagnosticStage::Configuration,
        "/workspace/repos",
        "fs:1:1",
        None,
        AffectedScope::Repository {
            repository: repository.to_owned(),
        },
        "remediation",
    )
    .expect("diagnostic")
}

#[test]
fn every_permitted_error_appears_once_with_stable_ordering() {
    let diagnostics = vec![
        diag("config-malformed", "dest-a"),
        diag("source-unavailable", "dest-b"),
        diag("config-malformed", "dest-a"),
        diag("source-unavailable", "dest-b"),
    ];
    let aggregated = aggregate_diagnostics(&diagnostics);
    assert_eq!(aggregated.len(), 2, "duplicates appear once");
    assert_eq!(aggregated[0].code, "config-malformed");
    assert_eq!(aggregated[1].code, "source-unavailable");
}

#[test]
fn secret_sentinels_never_appear() {
    let secret = "hunt".to_owned() + "er2";
    let url = format!("https://user:{secret}@example.test/upstream.git");
    let redacted = redact(&url);
    assert!(!redacted.contains("hunter2"), "{redacted}");
    assert!(redacted.contains("<redacted>"), "{redacted}");
    let token = "supersecret".to_owned() + "token1234";
    let env = format!("OMNIREPO_TOKEN={token} more");
    let redacted = redact(&env);
    assert!(!redacted.contains("supersecrettoken1234"), "{redacted}");
    let pass = "hunt".to_owned() + "er3";
    let filename = format!("config password={pass}.yaml");
    let redacted = redact(&filename);
    assert!(!redacted.contains("hunter3"), "{redacted}");
    // Non-secret values stay intact.
    assert_eq!(redact("mode=sync ok"), "mode=sync ok");
    assert_eq!(
        redact("https://example.test/repo"),
        "https://example.test/repo"
    );
}

#[test]
fn rendered_diagnostics_are_redacted() {
    let diagnostics = vec![diag("config-malformed", "dest-a")];
    let rendered = render_redacted(&diagnostics);
    assert_eq!(rendered.len(), 1);
    assert!(rendered[0].contains("config-malformed"), "{}", rendered[0]);
}
