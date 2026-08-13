//! Contract tests for the isolated Beads lifecycle transition matrix.
//!
//! The transition matrix is imported from the private tooling library. This
//! keeps integration tests on the same implementation as the dispatcher and
//! avoids compiling a duplicate source module for coverage.

use std::path::{Path, PathBuf};
use std::thread;

use serde_json::Value;

use omnirepo_dev::transition_matrix::{
    CASE_IDS, MatrixError, MatrixReport, TRANSITION_MATRIX_SCHEMA, run, run_with_br_path,
};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("developer-tool manifest is nested below the repository root")
        .to_path_buf()
}

fn assert_complete_report(report: &MatrixReport) {
    assert_eq!(report.schema, TRANSITION_MATRIX_SCHEMA);
    assert_eq!(report.cases.len(), CASE_IDS.len());
    assert_eq!(
        report
            .cases
            .iter()
            .map(|case| case.case_id.as_str())
            .collect::<Vec<_>>(),
        CASE_IDS
    );
    assert!(report.workspace_removed);
    assert!(report.cases.iter().all(|case| case.outcome.is_success()));

    let encoded = serde_json::to_value(report).expect("matrix report is JSON serializable");
    assert_eq!(encoded["schema"], TRANSITION_MATRIX_SCHEMA);
    assert_eq!(
        encoded["cases"].as_array().map(Vec::len),
        Some(CASE_IDS.len())
    );
    for case in encoded["cases"].as_array().expect("cases array") {
        assert!(case["case_id"].is_string());
        assert!(case["operation"].is_string());
        assert!(case["expected"].is_object());
        assert!(case["observed"].is_object());
        assert!(case["evidence"].is_string());
        assert!(case["outcome"].is_string());
    }
}

#[test]
fn frozen_transition_matrix_preserves_all_thirteen_cases() {
    let report = run(&repository_root()).expect("real br transition matrix should pass");
    assert_complete_report(&report);

    let case_ids = report
        .cases
        .iter()
        .map(|case| case.case_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(case_ids, CASE_IDS);

    let owner_close = report
        .cases
        .iter()
        .find(|case| case.case_id == "owner-close")
        .expect("owner-close case must remain in the frozen matrix");
    assert_eq!(
        owner_close.observed["close_reason"],
        "owner resolved decision"
    );
    assert_eq!(owner_close.observed["actor"], "thethracian");
    assert_eq!(owner_close.observed["dependent_ready"], true);

    let stale = report
        .cases
        .iter()
        .find(|case| case.case_id == "stale-export")
        .expect("stale-export case must remain in the frozen matrix");
    assert_eq!(stale.observed["db_status"], "in_progress");
    assert_eq!(stale.observed["jsonl_status"], "open");
    assert_eq!(stale.observed["workspace_health"], "degraded");
}

#[test]
fn concurrent_matrix_runs_use_disjoint_workspaces_and_cleanup() {
    let root = repository_root();
    thread::scope(|scope| {
        let first = scope.spawn(|| run(&root));
        let second = scope.spawn(|| run(&root));
        let first = first.join().expect("first matrix worker must not panic");
        let second = second.join().expect("second matrix worker must not panic");
        let first = first.expect("first matrix run should pass");
        let second = second.expect("second matrix run should pass");
        assert_complete_report(&first);
        assert_complete_report(&second);
    });
}

#[test]
fn missing_br_is_an_actionable_error_and_is_not_skipped() {
    let missing = repository_root().join("target/transition-matrix-missing-br");
    let error = run_with_br_path(&repository_root(), missing).expect_err("missing br must fail");
    assert!(matches!(error, MatrixError::MissingBr { .. }));
    assert!(error.to_string().contains("br"));
}

#[test]
fn nonzero_br_probe_is_an_actionable_error() {
    let probe = std::env::current_exe().expect("test executable path");
    let error = run_with_br_path(&repository_root(), probe).expect_err("bad br must fail");
    assert!(matches!(error, MatrixError::BrProbeFailed { .. }));
    assert!(error.to_string().contains("br"));
}

#[test]
fn report_does_not_leave_transition_matrix_temp_artifacts() {
    let report = run(&repository_root()).expect("matrix should pass");
    assert!(report.workspace_removed);
}

#[test]
fn report_json_contains_only_stable_case_data() {
    let report = run(&repository_root()).expect("matrix should pass");
    let value: Value = serde_json::to_value(report).expect("matrix report JSON");
    let serialized = serde_json::to_string(&value).expect("serialize matrix report");
    assert!(!serialized.contains("/tmp/omnirepo-transition-matrix-"));
    assert!(!serialized.contains("\"created_at\""));
    assert!(!serialized.contains("\"closed_at\""));
}
