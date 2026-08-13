use std::path::PathBuf;

use omnirepo_dev::run;
use omnirepo_dev::viewer::{ViewerAdapterError, ViewerCase, ViewerExport, adapt_export};

const EXPORT_FIXTURE: &str = include_str!("../../../tests/fixtures/viewer_export.json");

fn fixture() -> ViewerExport {
    serde_json::from_str(EXPORT_FIXTURE).expect("viewer export fixture must be valid JSON")
}

fn canonical_case_mut<'a>(export: &'a mut ViewerExport, name: &str) -> &'a mut ViewerCase {
    export
        .cases
        .iter_mut()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing fixture case {name}"))
}

fn expect_invalid_contract(export: ViewerExport, reason: &str) {
    let error = adapt_export(export).expect_err("invalid contract must fail closed");
    assert!(
        matches!(error, ViewerAdapterError::InvalidContract { .. }),
        "expected invalid contract, got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!("invalid viewer contract: {reason}")
    );
}

#[test]
fn validate_contract_rejects_every_declared_contract_mismatch() {
    let mut export = fixture();
    export.schema = "unsupported.viewer-export.v9".to_owned();
    let error = adapt_export(export).expect_err("schema mismatch must fail closed");
    assert!(matches!(error, ViewerAdapterError::SchemaMismatch { .. }));
    assert_eq!(
        error.to_string(),
        "unsupported viewer export schema: expected omnirepo.viewer-export-fixture.v1, got unsupported.viewer-export.v9"
    );

    let mut export = fixture();
    export.contract.canonical_actionable_sources.reverse();
    expect_invalid_contract(
        export,
        "canonical actionable sources must be br-ready then checked-agent-plan",
    );

    let mut export = fixture();
    export.contract.raw_bv = "actionable".to_owned();
    expect_invalid_contract(export, "raw bv must be advisory-only");

    let mut export = fixture();
    export.contract.owner_queue.action = "agent-only".to_owned();
    expect_invalid_contract(
        export,
        "owner queue must be owner-only, decision status, and label-gated",
    );

    let mut export = fixture();
    export.contract.owner_queue.status = "open".to_owned();
    expect_invalid_contract(
        export,
        "owner queue must be owner-only, decision status, and label-gated",
    );

    let mut export = fixture();
    export.contract.owner_queue.required_labels.clear();
    expect_invalid_contract(
        export,
        "owner queue must be owner-only, decision status, and label-gated",
    );

    let mut export = fixture();
    export.contract.closed_decision_provenance.pop();
    expect_invalid_contract(export, "closed decision provenance fields differ");

    let mut export = fixture();
    export.required_tracker_statuses.pop();
    expect_invalid_contract(
        export,
        "required tracker statuses and status classes differ",
    );

    let mut export = fixture();
    export.contract.wording.remove("actionable");
    expect_invalid_contract(export, "missing wording for category actionable");

    let mut export = fixture();
    export
        .contract
        .status_classes
        .insert("open".to_owned(), "missing-wording".to_owned());
    expect_invalid_contract(export, "status class has no declared wording");
}

#[test]
fn checked_plan_rejects_schema_trust_status_and_duplicate_sources() {
    let mut export = fixture();
    canonical_case_mut(&mut export, "tracer-decision-row")
        .sources
        .checked_plan
        .schema = "omnirepo.checked-agent-plan.v0".to_owned();
    let error = adapt_export(export).expect_err("checked schema mismatch must fail closed");
    assert!(matches!(
        error,
        ViewerAdapterError::InvalidCheckedPlan { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("expected omnirepo.checked-agent-plan.v1")
    );

    let mut export = fixture();
    canonical_case_mut(&mut export, "tracer-decision-row")
        .sources
        .checked_plan
        .evidence
        .raw_bv = "recommended".to_owned();
    let error = adapt_export(export).expect_err("untrusted checked plan must fail closed");
    assert!(matches!(
        error,
        ViewerAdapterError::InvalidCheckedPlan { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("raw bv evidence must be advisory-only")
    );

    let mut export = fixture();
    canonical_case_mut(&mut export, "tracer-decision-row")
        .sources
        .checked_plan
        .status = "error".to_owned();
    let error = adapt_export(export).expect_err("non-ok checked plan must fail closed");
    assert!(matches!(
        error,
        ViewerAdapterError::InvalidCheckedPlan { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("canonical cases require an ok checked plan")
    );

    let mut export = fixture();
    let case = canonical_case_mut(&mut export, "tracer-decision-row");
    case.sources.br_ready_ids.push("normal-work".to_owned());
    let error = adapt_export(export).expect_err("duplicate br source must fail closed");
    assert!(matches!(
        error,
        ViewerAdapterError::ActionableSourceMismatch { .. }
    ));

    let mut export = fixture();
    let case = canonical_case_mut(&mut export, "tracer-decision-row");
    case.sources
        .checked_plan
        .actionable_ids
        .push("normal-work".to_owned());
    let error = adapt_export(export).expect_err("duplicate checked source must fail closed");
    assert!(matches!(
        error,
        ViewerAdapterError::ActionableSourceMismatch { .. }
    ));
}

#[test]
fn canonical_source_ids_must_have_exactly_one_tracker_row() {
    let mut export = fixture();
    let case = canonical_case_mut(&mut export, "tracer-decision-row");
    case.sources.br_ready_ids.push("missing-row".to_owned());
    case.sources
        .checked_plan
        .actionable_ids
        .push("missing-row".to_owned());

    let error = adapt_export(export).expect_err("a source without a tracker row must fail closed");
    assert_eq!(
        error,
        ViewerAdapterError::SourceRowMismatch {
            case: "tracer-decision-row".to_owned(),
            id: "missing-row".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "canonical source id missing-row has no tracker row in viewer case tracer-decision-row"
    );
}

#[test]
fn duplicate_issue_ids_and_actionable_non_actionable_rows_fail_closed() {
    let mut export = fixture();
    let case = canonical_case_mut(&mut export, "tracer-decision-row");
    case.rows.push(case.rows[0].clone());
    let error = adapt_export(export).expect_err("duplicate tracker IDs must fail closed");
    assert!(matches!(error, ViewerAdapterError::DuplicateIssueId { .. }));
    assert_eq!(
        error.to_string(),
        "duplicate issue id decision-active in viewer case tracer-decision-row"
    );

    let mut export = fixture();
    let case = canonical_case_mut(&mut export, "tracer-decision-row");
    case.sources.br_ready_ids.push("decision-active".to_owned());
    case.sources
        .checked_plan
        .actionable_ids
        .push("decision-active".to_owned());
    let error = adapt_export(export)
        .expect_err("canonical source IDs that project to an owner decision must fail closed");
    assert!(matches!(
        error,
        ViewerAdapterError::ActionableSourceMismatch { .. }
    ));
    assert_eq!(
        error.to_string(),
        "canonical action sources disagree in viewer case tracer-decision-row"
    );
}

#[test]
fn unknown_statuses_are_invalid_and_never_actionable() {
    let mut export = fixture();
    canonical_case_mut(&mut export, "tracer-decision-row")
        .rows
        .push(omnirepo_dev::viewer::TrackerRow {
            id: "future-status".to_owned(),
            issue_type: "task".to_owned(),
            labels: Vec::new(),
            status: "future-status".to_owned(),
            created_at: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
        });

    let projection = adapt_export(export).expect("unknown status should project as invalid");
    let case = &projection.cases[0];
    let row = case
        .list
        .iter()
        .find(|row| row.id == "future-status")
        .expect("unknown row must be present");
    assert_eq!(row.category, "invalid");
    assert!(!row.actionable);
    assert_eq!(case.invalid_ids, vec!["future-status"]);
    assert!(
        !case
            .triage
            .actionable_ids
            .contains(&"future-status".to_owned())
    );
}

#[test]
fn projection_lookup_and_filter_misses_are_deterministic() {
    let projection = adapt_export(fixture()).expect("fixture projection must be valid");
    let case = projection
        .cases
        .iter()
        .find(|case| case.name == "all-tracker-statuses")
        .expect("all-status case must be projected");

    assert!(case.detail("does-not-exist").is_none());
    assert!(case.filter("").is_empty());
    assert!(case.filter("unknown-category").is_empty());
    assert_eq!(case.filter("actionable").len(), 1);
}

#[test]
fn closed_decision_requires_each_provenance_field_independently() {
    for missing_field in ["created_at", "created_by", "closed_at", "close_reason"] {
        let mut export = fixture();
        let row = canonical_case_mut(&mut export, "all-tracker-statuses")
            .rows
            .iter_mut()
            .find(|row| row.id == "closed-decision")
            .expect("closed decision fixture row must exist");
        match missing_field {
            "created_at" => row.created_at = None,
            "created_by" => row.created_by = None,
            "closed_at" => row.closed_at = None,
            "close_reason" => row.close_reason = None,
            _ => unreachable!("test matrix only contains known fields"),
        }

        let projection = adapt_export(export).expect("missing provenance remains valid input");
        let case = projection
            .cases
            .iter()
            .find(|case| case.name == "all-tracker-statuses")
            .expect("all-status case must be projected");
        let row = case
            .list
            .iter()
            .find(|row| row.id == "closed-decision")
            .expect("closed decision row must remain present");
        assert_eq!(row.category, "closed", "missing {missing_field}");
        assert!(!row.actionable, "missing {missing_field}");
        assert!(!case.owner_queue_ids.contains(&"closed-decision".to_owned()));
    }
}

#[test]
fn every_viewer_error_has_stable_display_and_no_source() {
    let errors = [
        ViewerAdapterError::SchemaMismatch {
            expected: "expected".to_owned(),
            actual: "actual".to_owned(),
        },
        ViewerAdapterError::InvalidContract {
            reason: "bad contract".to_owned(),
        },
        ViewerAdapterError::DuplicateIssueId {
            case: "case".to_owned(),
            id: "id".to_owned(),
        },
        ViewerAdapterError::ActionableSourceMismatch {
            case: "case".to_owned(),
        },
        ViewerAdapterError::SourceRowMismatch {
            case: "case".to_owned(),
            id: "id".to_owned(),
        },
        ViewerAdapterError::InvalidCheckedPlan {
            case: "case".to_owned(),
            reason: "bad plan".to_owned(),
        },
    ];
    let expected = [
        "unsupported viewer export schema: expected expected, got actual",
        "invalid viewer contract: bad contract",
        "duplicate issue id id in viewer case case",
        "canonical action sources disagree in viewer case case",
        "canonical source id id has no tracker row in viewer case case",
        "invalid checked plan in viewer case case: bad plan",
    ];

    for (error, expected) in errors.iter().zip(expected) {
        assert_eq!(error.to_string(), expected);
        assert!(std::error::Error::source(error).is_none());
    }
}

#[test]
fn viewer_cli_rejects_misuse_and_malformed_json_without_partial_output() {
    let cases = [
        (vec!["viewer"], "viewer requires the refresh subcommand"),
        (
            vec!["viewer", "unsupported"],
            "unsupported viewer command: unsupported",
        ),
        (
            vec!["viewer", "refresh", "--input"],
            "--input requires a path",
        ),
        (
            vec!["viewer", "refresh", "--unknown"],
            "unsupported viewer refresh argument: --unknown",
        ),
        (
            vec!["viewer", "refresh", "--json"],
            "viewer refresh requires --input PATH",
        ),
    ];
    for (arguments, expected) in cases {
        let output = run(arguments.clone());
        assert_eq!(output.status, 2, "arguments: {arguments:?}");
        assert!(output.stdout.is_empty());
        assert!(
            output.stderr.contains(expected),
            "diagnostic: {}",
            output.stderr
        );
    }

    let help = run(["viewer", "refresh", "--help"]);
    assert_eq!(help.status, 0);
    assert!(
        help.stdout
            .contains("omnirepo-dev: private repository tooling")
    );
    assert!(help.stderr.is_empty());

    let malformed = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let malformed = malformed.to_string_lossy().into_owned();
    let output = run(vec![
        "viewer".to_owned(),
        "refresh".to_owned(),
        "--input".to_owned(),
        malformed,
        "--json".to_owned(),
    ]);
    assert_eq!(output.status, 1);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains("viewer export is invalid JSON"));
    assert!(output.stderr.len() < 4_096);
}
