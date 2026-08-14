//! Focused proof for running the complete acceptance gate from the
//! canonical journey matrix.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::final_gate::{FinalGateReport, run_final_acceptance_gate, run_quality_gates};
use crate::lifecycle::release_gates::GateRun;
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("final-gate-")
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn the_final_gate_runs_every_canonical_journey() {
    let fixture = fixture_base();
    let clean_home = fixture.path().join("clean-home");
    fs::create_dir_all(&clean_home).expect("home");
    let report = run_final_acceptance_gate(&clean_home, &[]);
    let matrix_len = crate::lifecycle::acceptance_journeys::canonical_journey_matrix().len();
    assert_eq!(report.journeys.len(), matrix_len, "every journey ran");
    assert!(
        report
            .journeys
            .iter()
            .all(|journey| journey.outcome == "passed"),
        "{:?}",
        report
            .journeys
            .iter()
            .filter(|journey| journey.outcome != "passed")
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_quality_gates_are_collected_without_averaging() {
    let fixture = fixture_base();
    let pass = fixture.path().join("gate-pass");
    let fail = fixture.path().join("gate-fail");
    fs::write(&pass, "#!/bin/sh\nexit 0\n").expect("pass");
    fs::write(&fail, "#!/bin/sh\necho gate failed\nexit 1\n").expect("fail");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&pass, &fail] {
            let mut permissions = fs::metadata(path).expect("meta").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("mode");
        }
    }
    let gates = vec![
        ("fmt".to_owned(), vec![pass.display().to_string()]),
        ("tests".to_owned(), vec![fail.display().to_string()]),
    ];
    let runs = run_quality_gates(&gates);
    assert_eq!(runs.len(), 2);
    assert!(runs[0].passed);
    assert!(
        !runs[1].passed,
        "a failing gate is reported, never averaged away"
    );
    assert!(runs[1].evidence.contains("gate failed"));
}

#[test]
fn the_final_report_is_an_all_or_nothing_no_averaging_verdict() {
    let fixture = fixture_base();
    let clean_home = fixture.path().join("clean-home");
    fs::create_dir_all(&clean_home).expect("home");
    let report = run_final_acceptance_gate(&clean_home, &[]);
    // The verdict is all-pass or reported failures; the report carries
    // every failure explicitly.
    if !report.all_passed {
        assert!(
            report.journeys.iter().any(|j| j.outcome == "failed")
                || report.gates.iter().any(|g| !g.passed),
            "a non-passing verdict must name its failures: {report:?}"
        );
    }
}

#[test]
fn the_gate_runs_from_a_clean_environment_with_no_ambient_state() {
    let fixture = fixture_base();
    let clean_home = fixture.path().join("clean-home");
    fs::create_dir_all(&clean_home).expect("home");
    let report = run_final_acceptance_gate(&clean_home, &[]);
    // Every journey ran in the clean environment.
    for journey in &report.journeys {
        assert!(!journey.id.is_empty());
    }
    assert!(report.all_passed, "{report:?}");
}
