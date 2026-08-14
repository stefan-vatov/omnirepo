//! Focused proof for the direct-output prohibition guard.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::output_guard::{
    DirectOutput, assert_no_direct_output, is_projection_boundary,
};
use std::path::Path;

fn product_sources() -> Vec<(String, String)> {
    // The product source tree (test files are scanned separately).
    let mut sources = Vec::new();
    collect(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources
}

fn collect(directory: std::path::PathBuf, sources: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(&directory).expect("read") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect(path, sources);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let name = path.display().to_string();
            let content = std::fs::read_to_string(&path).expect("content");
            sources.push((name, content));
        }
    }
}

#[test]
fn no_worker_or_adapter_writes_stdout_or_stderr_directly() {
    let violations = assert_no_direct_output(&product_sources());
    let worker_violations = violations
        .iter()
        .filter(|violation| !is_projection_boundary(&violation.path))
        .collect::<Vec<_>>();
    assert!(
        worker_violations.is_empty(),
        "workers write directly: {:?}",
        worker_violations
    );
}

#[test]
fn the_projection_boundary_is_the_only_allowed_direct_output_site() {
    // The CLI invocation boundary is the single allowed direct-output
    // site (its stderr diagnostics follow the decided stream contract).
    let boundary = "src/lifecycle/invocation.rs";
    assert!(is_projection_boundary(boundary));
    assert!(!is_projection_boundary("src/lifecycle/agent_runtime.rs"));
    assert!(!is_projection_boundary("src/lifecycle/check_runner.rs"));
    assert!(!is_projection_boundary("src/lifecycle/fleet_fanout.rs"));
}

#[test]
fn the_cli_surface_carries_no_legacy_output_flags() {
    // The .27 surface: no verbose, progress, table, or logger flags.
    let cli = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/configuration/mod.rs"),
    )
    .expect("cli");
    for forbidden in ["verbose", "progress", "table", "--log", "logger"] {
        assert!(
            !cli.contains(&format!("#[arg(long)]\n    {forbidden}"))
                && !cli.contains(&format!("{forbidden}:"))
                && !cli.contains(&format!("{forbidden} =")),
            "legacy flag {forbidden:?} remains in the CLI"
        );
    }
    // Only the decided output selector exists.
    assert!(
        cli.contains("output"),
        "the decided --output selector exists"
    );
    assert!(
        !cli.contains("OutputMode::Json") || cli.contains("--output"),
        "{cli}"
    );
}

#[test]
fn raw_evidence_stays_in_the_record_not_the_streams() {
    // The projections never carry raw evidence: the guard treats any
    // direct evidence emission as a violation.
    let fake_worker = vec![(
        "src/lifecycle/worker.rs".to_owned(),
        "fn work() { println!(\"{}\", evidence); }".to_owned(),
    )];
    let violations = assert_no_direct_output(&fake_worker);
    assert_eq!(violations.len(), 1, "{violations:?}");
    let fake_boundary = vec![(
        "src/lifecycle/invocation.rs".to_owned(),
        "eprintln!(\"diagnostic\");".to_owned(),
    )];
    assert!(assert_no_direct_output(&fake_boundary).is_empty());
}
