use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;
use serde_json::Value;

use super::beads_validator::{
    FindingCode, MAX_DIAGNOSTIC_TEXT, MAX_FINDINGS, ValidationStatus, ValidatorError,
    validate_contents, validate_path,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Deserialize)]
struct FixtureCase {
    tracked_lines: Vec<String>,
    #[serde(default)]
    omit_tracked: bool,
}

fn case(name: &str) -> FixtureCase {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/beads_contract/cases")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(path).expect("read frozen validator case"))
        .expect("parse frozen validator case")
}

fn fixture_path(name: &str) -> PathBuf {
    let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "omnirepo-dev-validator-test-{}-{name}-{number}",
        std::process::id()
    ))
}

fn report_for(name: &str) -> super::beads_validator::ValidationReport {
    let fixture = case(name);
    let root = fixture_path(name);
    let path = root.join(".beads/issues.jsonl");
    if !fixture.omit_tracked {
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture");
        let mut contents = fixture.tracked_lines.join("\n");
        if !contents.is_empty() {
            contents.push('\n');
        }
        fs::write(&path, contents).expect("write fixture");
    }
    let report = validate_path(&path)
        .map_err(|error| error.into_report())
        .unwrap_or_else(|report| report);
    if root.exists() {
        fs::remove_dir_all(root).expect("remove fixture");
    }
    report
}

#[test]
fn valid_frozen_cases_accept_active_closed_history_and_ordinary_states() {
    for name in ["validator-valid-basic", "validator-valid-matrix"] {
        let report = report_for(name);
        assert_eq!(
            report.status,
            ValidationStatus::Consistent,
            "case {name}: {:?}",
            report.findings
        );
        assert!(
            report.findings.is_empty(),
            "case {name}: {:?}",
            report.findings
        );
        assert!(!report.truncated);
    }
}

#[test]
fn invalid_frozen_matrix_has_stable_codes_issue_ids_and_lines() {
    let report = report_for("validator-invalid-matrix");
    let expected = [
        (
            1,
            Some("decision-missing-labels"),
            FindingCode::ActiveDecisionLabelsMissing,
        ),
        (
            2,
            Some("decision-one-label"),
            FindingCode::ActiveDecisionLabelsMissing,
        ),
        (
            3,
            Some("reopened-open"),
            FindingCode::DecisionLabelsRequireDecision,
        ),
        (
            4,
            Some("claimed-in-progress"),
            FindingCode::DecisionLabelsRequireDecision,
        ),
        (
            5,
            Some("label-only-drift"),
            FindingCode::DecisionLabelsRequireDecision,
        ),
        (
            6,
            Some("ordinary-entered-decision"),
            FindingCode::ActiveDecisionLabelsMissing,
        ),
        (
            7,
            Some("closed-missing-created-at"),
            FindingCode::ClosedDecisionProvenanceMissing,
        ),
        (
            8,
            Some("closed-missing-created-by"),
            FindingCode::ClosedDecisionProvenanceMissing,
        ),
        (
            9,
            Some("closed-missing-closed-at"),
            FindingCode::ClosedDecisionProvenanceMissing,
        ),
        (
            10,
            Some("closed-missing-close-reason"),
            FindingCode::ClosedDecisionProvenanceMissing,
        ),
        (
            11,
            Some("closed-one-label"),
            FindingCode::ClosedDecisionLabelsMissing,
        ),
        (13, Some("duplicate-id"), FindingCode::DuplicateIssueId),
        (14, None, FindingCode::MalformedJsonRecord),
        (15, None, FindingCode::RecordNotObject),
        (16, Some("labels-object"), FindingCode::LabelsNotArray),
        (17, Some("labels-number"), FindingCode::LabelNotString),
        (18, Some("duplicate-label"), FindingCode::DuplicateLabel),
        (
            18,
            Some("duplicate-label"),
            FindingCode::DecisionLabelsRequireDecision,
        ),
        (19, Some("unknown-status"), FindingCode::UnknownStatus),
        (20, None, FindingCode::BlankJsonlRecord),
    ];

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert_eq!(
        report.findings.len(),
        expected.len(),
        "{:?}",
        report.findings
    );
    for (finding, (line, issue_id, code)) in report.findings.iter().zip(expected) {
        assert_eq!(finding.line, Some(line));
        assert_eq!(
            finding.issue_id.as_ref().map(|value| value.as_str()),
            issue_id
        );
        assert_eq!(finding.code, code);
    }
}

#[test]
fn missing_and_empty_exports_fail_closed_with_stable_findings() {
    let missing = report_for("validator-invalid-missing-export");
    assert_eq!(missing.status, ValidationStatus::Invalid);
    assert_eq!(missing.findings[0].code, FindingCode::TrackedJsonlMissing);
    assert_eq!(missing.findings[0].line, None);

    let empty = report_for("validator-invalid-empty-export");
    assert_eq!(empty.status, ValidationStatus::Invalid);
    assert_eq!(empty.findings[0].code, FindingCode::TrackedJsonlEmpty);
    assert_eq!(empty.findings[0].line, None);
}

#[test]
fn validation_is_read_only_and_json_serializable() {
    let fixture = case("validator-valid-matrix");
    let root = fixture_path("read-only");
    let path = root.join(".beads/issues.jsonl");
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture");
    let contents = fixture.tracked_lines.join("\n") + "\n";
    fs::write(&path, &contents).expect("write fixture");

    let report = validate_path(&path).expect("validate fixture");
    assert_eq!(
        serde_json::to_value(&report).expect("serialize report")["schema"],
        Value::String("omnirepo.decision-validation.v1".to_owned())
    );
    assert_eq!(fs::read_to_string(&path).expect("read fixture"), contents);
    if root.exists() {
        fs::remove_dir_all(root).expect("remove fixture");
    }
}

#[test]
fn diagnostics_are_bounded_without_hiding_the_first_findings() {
    let contents = (0..(MAX_FINDINGS + 10))
        .map(|index| format!(r#"{{"id":"bad-{index}","status":"unknown"}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    let report = validate_contents(Path::new("fixture.jsonl"), &contents);

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert_eq!(report.findings.len(), MAX_FINDINGS);
    assert!(report.truncated);
    assert_eq!(report.findings[0].line, Some(1));
    assert!(report.findings.iter().all(|finding| {
        finding.message.len() <= MAX_DIAGNOSTIC_TEXT && finding.code == FindingCode::UnknownStatus
    }));

    let long_path = "é".repeat(200);
    let bounded = validate_contents(Path::new(&long_path), "");
    assert!(bounded.path.len() <= MAX_DIAGNOSTIC_TEXT);
    assert!(bounded.path.ends_with('…'));
}

#[test]
fn path_faults_preserve_paths_and_project_stable_diagnostics() {
    let root = fixture_path("path-faults");
    fs::create_dir_all(&root).expect("create directory fixture");

    let directory_error = validate_path(&root).expect_err("directory must not be read as JSONL");
    assert!(matches!(directory_error, ValidatorError::Io { .. }));
    assert_eq!(directory_error.path(), root.as_path());
    assert!(
        directory_error
            .to_string()
            .contains("cannot read tracked JSONL")
    );
    assert!(directory_error.source().is_some());
    let directory_report = directory_error.into_report();
    assert_eq!(directory_report.status, ValidationStatus::Invalid);
    assert_eq!(directory_report.path, root.display().to_string());
    assert_eq!(
        directory_report.findings[0].code,
        FindingCode::TrackedJsonlUnreadable
    );

    let invalid_utf8_path = root.join("invalid-utf8.jsonl");
    fs::write(&invalid_utf8_path, [0x7b, 0xff, 0x7d]).expect("write invalid UTF-8 fixture");
    let invalid_utf8_error =
        validate_path(&invalid_utf8_path).expect_err("invalid UTF-8 must fail closed");
    assert!(matches!(
        invalid_utf8_error,
        ValidatorError::InvalidUtf8 { .. }
    ));
    assert_eq!(invalid_utf8_error.path(), invalid_utf8_path.as_path());
    assert!(
        invalid_utf8_error
            .to_string()
            .contains("tracked JSONL is not valid UTF-8")
    );
    assert!(invalid_utf8_error.source().is_none());
    let invalid_utf8_report = invalid_utf8_error.into_report();
    assert_eq!(invalid_utf8_report.status, ValidationStatus::Invalid);
    assert_eq!(
        invalid_utf8_report.path,
        invalid_utf8_path.display().to_string()
    );
    assert_eq!(
        invalid_utf8_report.findings[0].code,
        FindingCode::TrackedJsonlInvalidUtf8
    );

    assert_eq!(FindingCode::MissingIssueId.to_string(), "missing-issue-id");
    assert_eq!(
        FindingCode::TrackedJsonlUnreadable.to_string(),
        "tracked-jsonl-unreadable"
    );

    fs::remove_dir_all(root).expect("remove directory fixture");
}

#[test]
fn missing_issue_ids_are_rejected_for_missing_empty_and_non_string_values() {
    let contents = [
        r#"{"status":"open"}"#,
        r#"{"id":"","status":"open"}"#,
        r#"{"id":42,"status":"open"}"#,
    ]
    .join("\n");
    let report = validate_contents(Path::new("missing-ids.jsonl"), &contents);

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert_eq!(report.findings.len(), 3);
    for (finding, expected_line) in report.findings.iter().zip(1..=3) {
        assert_eq!(finding.code, FindingCode::MissingIssueId);
        assert_eq!(finding.line, Some(expected_line));
        assert!(finding.issue_id.is_none());
        assert_eq!(finding.message, "id must be a non-empty string");
    }
}

#[test]
fn present_empty_closed_decision_provenance_is_rejected_per_field() {
    for field in ["created_at", "created_by", "closed_at", "close_reason"] {
        let mut object = serde_json::json!({
            "id": format!("empty-{field}"),
            "status": "closed",
            "labels": ["decision-needed", "human-input"],
            "created_at": "2026-08-13T00:00:00Z",
            "created_by": "owner",
            "closed_at": "2026-08-13T00:01:00Z",
            "close_reason": "owner decision recorded",
        });
        object[field] = Value::String(String::new());

        let report = validate_contents(Path::new("empty-provenance.jsonl"), &object.to_string());

        assert_eq!(report.status, ValidationStatus::Invalid, "field={field}");
        assert_eq!(report.findings.len(), 1, "field={field}");
        let finding = &report.findings[0];
        assert_eq!(finding.code, FindingCode::ClosedDecisionProvenanceMissing);
        assert_eq!(
            finding.issue_id.as_ref().map(|id| id.as_str()),
            Some(object["id"].as_str().unwrap())
        );
        assert_eq!(
            finding.message,
            format!("closed decision is missing provenance fields: {field}")
        );
    }
}
