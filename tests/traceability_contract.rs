#[path = "traceability/mod.rs"]
mod traceability;

use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn matrix() -> String {
    std::fs::read_to_string(root().join("tests/traceability/matrix.json"))
        .expect("read canonical traceability matrix")
}

fn beads() -> String {
    std::fs::read_to_string(root().join(".beads/issues.jsonl")).expect("read tracked Beads export")
}

fn first_row(source: &str) -> &str {
    source
        .lines()
        .find(|line| {
            line.trim_start()
                .starts_with("{\"id\":\"principle-managed-authoritative\"")
        })
        .expect("matrix must contain the first canonical row")
        .trim_end_matches(',')
}

fn append_row(source: &str, row: &str) -> String {
    let marker = "\n  ]\n}";
    let insertion = format!(",\n    {row}\n  ]\n}}");
    source
        .rfind(marker)
        .map(|index| {
            let mut output = source.to_owned();
            output.replace_range(index.., &insertion);
            output
        })
        .expect("matrix rows must have a final closing marker")
}

fn change_bead_status(source: &str, id: &str, from: &str, to: &str) -> String {
    let needle = format!("\"id\":\"{id}\"");
    let old_status = format!("\"status\":\"{from}\"");
    let new_status = format!("\"status\":\"{to}\"");
    source
        .lines()
        .map(|line| {
            if line.contains(&needle) {
                line.replacen(&old_status, &new_status, 1)
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn implemented_first_row(source: &str, verified: bool, accepted_downstream: bool) -> String {
    let implementation = "omni-constitutional-convergence-2r9.64.1";
    let downstream = if accepted_downstream {
        "omni-test-acceptance-1"
    } else {
        "omni-constitutional-convergence-2r9.32.5"
    };
    let planned_test = "{\"role\":\"planned\",\"contract\":\"omni-constitutional-convergence-2r9.6#trace.principle.managed-authoritative\"}";
    let executable_test = "{\"role\":\"executable\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"canonical_traceability_matrix_is_valid\"}";
    let planned_evidence = "{\"role\":\"planned\",\"contract\":\"omni-constitutional-convergence-2r9.32.5#evidence.trace.principle.managed-authoritative.v1\"}";
    let artifact_evidence = "{\"role\":\"artifact\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"evidence.trace.principle.managed-authoritative.v1\"}";
    let planned_fixture = "\"fixture\":\"fixture:managed-whole-and-partial\"";
    let executable_fixture = "\"fixture\":\"fixture:managed-whole-and-partial\",\"fixture_locator\":{\"role\":\"fixture\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"fixture:managed-whole-and-partial\"}";
    let mut output = source
        .replacen(
            "\"primary_owner\":\"omni-constitutional-convergence-2r9.6\"",
            &format!("\"primary_owner\":\"{implementation}\""),
            1,
        )
        .replacen(
            "\"implementation_bead\":\"omni-constitutional-convergence-2r9.6\"",
            &format!("\"implementation_bead\":\"{implementation}\""),
            1,
        )
        .replacen(
            "\"downstream_bead\":\"omni-constitutional-convergence-2r9.32.5\"",
            &format!("\"downstream_bead\":\"{downstream}\""),
            1,
        )
        .replacen(
            "\"implementation_status\":\"specified\"",
            "\"implementation_status\":\"implemented\"",
            1,
        )
        .replacen(planned_test, executable_test, 1)
        .replacen(planned_fixture, executable_fixture, 1);
    if verified {
        output = output
            .replacen(
                "\"verification_status\":\"specified\"",
                "\"verification_status\":\"verified\"",
                1,
            )
            .replacen(planned_evidence, artifact_evidence, 1);
    }
    output
}

fn append_synthetic_acceptance_bead(source: &str, evidence: bool) -> String {
    let evidence_field = if evidence {
        ",\"traceability_evidence\":[{\"schema\":\"omnirepo.traceability-evidence.v1\",\"row_id\":\"principle-managed-authoritative\",\"case_id\":\"trace.principle.managed-authoritative\",\"evidence_id\":\"evidence.trace.principle.managed-authoritative.v1\",\"locator_role\":\"artifact\",\"downstream_bead\":\"omni-test-acceptance-1\"}]"
    } else {
        ""
    };
    format!(
        "{source}\n{{\"id\":\"omni-test-acceptance-1\",\"status\":\"closed\",\"issue_type\":\"task\",\"labels\":[],\"created_at\":\"2026-08-13T00:00:00Z\",\"created_by\":\"test\",\"closed_at\":\"2026-08-13T00:01:00Z\",\"close_reason\":\"structured acceptance evidence\",\"notes\":\"generic acceptance note\"{evidence_field}}}"
    )
}

fn file_backed_fixture(
    executable_source: &str,
    executable_selector: &str,
    fixture_source: &str,
    evidence_source: &str,
) -> (tempfile::TempDir, traceability::Report) {
    let directory = tempfile::tempdir_in(root().join("tests/traceability"))
        .expect("create contained file-backed validation fixture");
    let executable = directory.path().join("executable.rs");
    let fixture = directory.path().join("fixture.json");
    let evidence = directory.path().join("evidence.json");
    let matrix_file = directory.path().join("matrix.json");
    let beads_file = directory.path().join("issues.jsonl");
    fs::write(&executable, executable_source).expect("write executable fixture");
    fs::write(&fixture, fixture_source).expect("write structured fixture record");
    fs::write(&evidence, evidence_source).expect("write structured evidence record");
    fs::write(
        &beads_file,
        append_synthetic_acceptance_bead(&beads(), true),
    )
    .expect("write synthetic Beads export");

    let relative = |path: &Path| {
        path.strip_prefix(root())
            .expect("fixture remains repository-contained")
            .to_string_lossy()
            .into_owned()
    };
    let mut source = implemented_first_row(&matrix(), true, true);
    let old_test = "{\"role\":\"executable\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"canonical_traceability_matrix_is_valid\"}";
    let new_test = format!(
        "{{\"role\":\"executable\",\"path\":\"{}\",\"selector\":\"{executable_selector}\"}}",
        relative(&executable)
    );
    source = source.replacen(old_test, &new_test, 1);
    let old_fixture = "{\"role\":\"fixture\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"fixture:managed-whole-and-partial\"}";
    let new_fixture = format!(
        "{{\"role\":\"fixture\",\"path\":\"{}\",\"selector\":\"fixture:managed-whole-and-partial\"}}",
        relative(&fixture)
    );
    source = source.replacen(old_fixture, &new_fixture, 1);
    let old_evidence = "{\"role\":\"artifact\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"evidence.trace.principle.managed-authoritative.v1\"}";
    let new_evidence = format!(
        "{{\"role\":\"artifact\",\"path\":\"{}\",\"selector\":\"evidence.trace.principle.managed-authoritative.v1\"}}",
        relative(&evidence)
    );
    source = source.replacen(old_evidence, &new_evidence, 1);
    fs::write(&matrix_file, source).expect("write matrix fixture");
    let report = traceability::validate_file_with_beads(&matrix_file, &beads_file)
        .expect("file-backed matrix fixture must be readable");
    (directory, report)
}

#[test]
fn canonical_traceability_matrix_is_valid() {
    let report = traceability::validate_file(&root().join("tests/traceability/matrix.json"))
        .expect("canonical traceability matrix must be readable");
    assert!(report.valid, "findings: {:?}", report.findings);
    assert_eq!(report.rows, 57);
    assert_eq!(report.replay_id, "traceability-validator.v1");
    assert!(!report.truncated);
}

#[test]
fn duplicate_primary_owner_is_rejected_without_collapsing_the_other_rows() {
    let source = matrix();
    let report = traceability::validate_source(&append_row(&source, first_row(&source)), &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-primary-owner")
    );
    assert_eq!(report.rows, 58);
}

#[test]
fn missing_mandatory_clause_is_rejected() {
    let source = matrix()
        .lines()
        .filter(|line| {
            !line
                .trim_start()
                .starts_with("{\"id\":\"principle-managed-authoritative\"")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let report = traceability::validate_source(&source, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "missing-required-row")
    );
}

#[test]
fn stale_orphan_bead_references_are_rejected() {
    let source = matrix().replace(
        "omni-constitutional-convergence-2r9.6",
        "omni-stale-bead-999",
    );
    let report = traceability::validate_source(&source, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "orphan-bead")
    );
}

#[test]
fn duplicate_row_ids_are_distinct_from_duplicate_primary_ownership() {
    let source = matrix().replace(
        "\"id\":\"principle-convention-intent\"",
        "\"id\":\"principle-managed-authoritative\"",
    );
    let report = traceability::validate_source(&source, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-row-id")
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-primary-owner")
    );
}

#[test]
fn policy_selecting_values_are_rejected() {
    let source = matrix().replace(
        "{\"id\":\"principle-managed-authoritative\",",
        "{\"id\":\"principle-managed-authoritative\",\"selected_value\":\"invented-policy\",",
    );
    let report = traceability::validate_source(&source, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "policy-value-forbidden")
    );
}

#[test]
fn malformed_schema_and_missing_effects_are_rejected() {
    let malformed = traceability::validate_source("{", &beads());
    assert!(!malformed.valid);
    assert!(
        malformed
            .findings
            .iter()
            .any(|finding| finding.code == "schema-malformed")
    );

    let missing_effect = matrix().replace(
        "\"expected_effect\":\"positive\"",
        "\"expected_effect\":\"unknown\"",
    );
    let report = traceability::validate_source(&missing_effect, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "schema-value")
    );
}

#[test]
fn diagnostics_have_stable_replay_ids_and_bounded_output() {
    let rows = (0..100).map(|_| "null").collect::<Vec<_>>().join(",");
    let source = format!("{{\"rows\":[{rows}]}}");
    let report = traceability::validate_source(&source, "not-an-object\n");
    assert!(!report.valid);
    assert!(report.truncated);
    assert_eq!(report.findings.len(), 64);
    for finding in &report.findings {
        assert!(finding.replay_id.starts_with("traceability/"));
        assert!(finding.path.len() <= 256);
        assert!(finding.message.len() <= 256);
        assert!(finding.path.is_char_boundary(finding.path.len()));
        assert!(finding.message.is_char_boundary(finding.message.len()));
    }
    let inserted = source.replacen("{\"rows\"", "{\"extra\":true,\"rows\"", 1);
    let inserted_report = traceability::validate_source(&inserted, "not-an-object\n");
    let original = report
        .findings
        .iter()
        .find(|finding| finding.path == "root.schema")
        .expect("base report has a stable root finding");
    let replayed = inserted_report
        .findings
        .iter()
        .find(|finding| finding.path == "root.schema")
        .expect("inserted report retains the root finding");
    assert_eq!(original.replay_id, replayed.replay_id);
}

#[test]
fn missing_matrix_file_is_an_io_failure() {
    let error = traceability::validate_file(&root().join("tests/traceability/no-such-matrix.json"))
        .expect_err("missing matrix must not be treated as an empty valid matrix");
    assert!(
        error
            .to_string()
            .contains("cannot inspect traceability matrix")
    );
}

#[test]
fn strict_json_rejects_yaml_and_duplicate_keys() {
    let yaml = traceability::validate_source("schema: omnirepo.traceability-matrix.v1\n", &beads());
    assert!(
        yaml.findings
            .iter()
            .any(|finding| finding.code == "schema-malformed")
    );

    let duplicate = traceability::validate_source("{\"rows\":[],\"rows\":[]}", &beads());
    assert!(
        duplicate
            .findings
            .iter()
            .any(|finding| finding.code == "schema-malformed")
    );

    let malformed_bead = traceability::validate_source(&matrix(), "id: not-json\n");
    assert!(
        malformed_bead
            .findings
            .iter()
            .any(|finding| finding.code == "bead-export-malformed")
    );
}

#[test]
fn taxonomy_is_a_closed_set() {
    let source = matrix().replacen(
        "[\"unit\", \"component\", \"black-box-e2e\", \"adversarial\", \"platform\", \"scale\", \"optional\"]",
        "[\"unit\", \"component\", \"black-box-e2e\", \"adversarial\", \"platform\", \"scale\", \"optional\", \"unsupported\"]",
        1,
    );
    let report = traceability::validate_source(&source, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "schema-unexpected")
    );
}

#[test]
fn constitution_anchors_are_checked_against_the_source() {
    let constitution = std::fs::read_to_string(root().join("CONSTITUTION.md")).unwrap();
    let altered = constitution.replacen("## Boundaries", "## Altered", 1);
    let report = traceability::validate_source_with_constitution(&matrix(), &beads(), &altered);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "constitution-anchor-missing")
    );
}

#[test]
fn owner_decision_refs_require_closed_owner_provenance() {
    let bad = change_bead_status(
        &beads(),
        "omni-constitutional-convergence-2r9.19",
        "closed",
        "open",
    );
    let report = traceability::validate_source(&matrix(), &bad);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "owner-decision-not-closed")
    );

    let non_decision = matrix().replacen(
        "\"owner_decision_refs\":[\"omni-constitutional-convergence-2r9.19\",\"omni-constitutional-convergence-2r9.20\"]",
        "\"owner_decision_refs\":[\"omni-constitutional-convergence-2r9.6\"]",
        1,
    );
    let report = traceability::validate_source(&non_decision, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "owner-decision-labels-missing")
    );
}

#[test]
fn implementation_status_cannot_claim_future_work() {
    // Flip the fleet-progress row (its implementation bead .74 is still
    // open) to "implemented": the validator must flag the overclaim.
    let source = matrix().replacen(
        "\"implementation_bead\":\"omni-constitutional-convergence-2r9.59\",\"implementation_status\":\"specified\"",
        "\"implementation_bead\":\"omni-constitutional-convergence-2r9.59\",\"implementation_status\":\"implemented\"",
        1,
    );
    assert_ne!(source, matrix(), "the fixture must change the matrix");
    let report = traceability::validate_source(&source, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "implementation-status-overclaim")
    );
}

#[test]
fn identity_and_silence_rules_are_bidirectional() {
    let duplicate_fixture = matrix().replacen(
        "\"fixture\":\"fixture:absent-and-present-repository-policy\"",
        "\"fixture\":\"fixture:managed-whole-and-partial\"",
        1,
    );
    let report = traceability::validate_source(&duplicate_fixture, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-fixture")
    );

    let duplicate_case = matrix().replacen(
        "\"case_id\":\"trace.principle.convention-intent\"",
        "\"case_id\":\"trace.principle.managed-authoritative\"",
        1,
    );
    let report = traceability::validate_source(&duplicate_case, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-case-id")
    );

    let duplicate_evidence = matrix().replacen(
        "\"evidence_id\":\"evidence.trace.principle.convention-intent.v1\"",
        "\"evidence_id\":\"evidence.trace.principle.managed-authoritative.v1\"",
        1,
    );
    let report = traceability::validate_source(&duplicate_evidence, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-evidence-id")
    );

    let duplicate_replay = matrix().replacen(
        "\"replay_id\":\"replay.trace.principle.convention-intent.v1\"",
        "\"replay_id\":\"replay.trace.principle.managed-authoritative.v1\"",
        1,
    );
    let report = traceability::validate_source(&duplicate_replay, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate-replay-id")
    );

    let optional_false = matrix().replacen(
        "\"constitutional_silence\":true",
        "\"constitutional_silence\":false",
        1,
    );
    let report = traceability::validate_source(&optional_false, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "silence-not-explicit")
    );

    let required_true = matrix().replacen(
        "\"constitutional_silence\":false",
        "\"constitutional_silence\":true",
        1,
    );
    let report = traceability::validate_source(&required_true, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "silence-not-explicit")
    );
}

#[test]
fn specified_rows_use_unique_non_executable_contract_locators() {
    let report = traceability::validate_source(&matrix(), &beads());
    assert!(report.valid, "findings: {:?}", report.findings);

    let mismatched = matrix().replacen(
        "omni-constitutional-convergence-2r9.6#trace.principle.managed-authoritative",
        "omni-constitutional-convergence-2r9.6#trace.other",
        1,
    );
    let report = traceability::validate_source(&mismatched, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "locator-contract-mismatch")
    );

    let executable_plan = matrix().replacen(
        "{\"role\":\"planned\",\"contract\":\"omni-constitutional-convergence-2r9.6#trace.principle.managed-authoritative\"}",
        "{\"role\":\"executable\",\"path\":\"tests/traceability_contract.rs\",\"selector\":\"canonical_traceability_matrix_is_valid\"}",
        1,
    );
    let report = traceability::validate_source(&executable_plan, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "locator-role-mismatch")
    );
}

#[test]
fn implemented_rows_require_executable_test_locators() {
    let source = implemented_first_row(&matrix(), false, false);
    let report = traceability::validate_source(&source, &beads());
    assert!(report.valid, "findings: {:?}", report.findings);

    let unresolved_selector = ["missing", "runtime", "selector"].join("_");
    let unresolved = source.replacen(
        "canonical_traceability_matrix_is_valid",
        &unresolved_selector,
        1,
    );
    let report = traceability::validate_source(&unresolved, &beads());
    assert!(
        report.valid,
        "pure validation must not claim filesystem evidence"
    );
}

#[test]
fn verified_rows_require_closed_acceptance_and_evidence_proof() {
    let source = implemented_first_row(&matrix(), true, false);
    let report = traceability::validate_source(&source, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification-downstream-not-accepted")
    );

    let accepted = implemented_first_row(&matrix(), true, true);
    let accepted_beads = append_synthetic_acceptance_bead(&beads(), true);
    let report = traceability::validate_source(&accepted, &accepted_beads);
    assert!(report.valid, "findings: {:?}", report.findings);
}

#[test]
fn executable_locators_and_fixture_resolve_in_file_validation() {
    let (_directory, report) = file_backed_fixture(
        "fn exact_traceability_case() {}\n",
        "exact_traceability_case",
        r#"{"schema":"omnirepo.traceability-fixture.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","fixture_id":"fixture:managed-whole-and-partial","locator_role":"fixture","downstream_bead":"omni-test-acceptance-1"}"#,
        r#"{"schema":"omnirepo.traceability-evidence.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","evidence_id":"evidence.trace.principle.managed-authoritative.v1","locator_role":"artifact","downstream_bead":"omni-test-acceptance-1"}"#,
    );
    assert!(report.valid, "findings: {:?}", report.findings);
}

#[test]
fn executable_locators_resolve_exact_nested_module_paths() {
    let (_directory, report) = file_backed_fixture(
        "mod nested { fn exact_traceability_case() {} }\n",
        "nested::exact_traceability_case",
        r#"{"schema":"omnirepo.traceability-fixture.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","fixture_id":"fixture:managed-whole-and-partial","locator_role":"fixture","downstream_bead":"omni-test-acceptance-1"}"#,
        r#"{"schema":"omnirepo.traceability-evidence.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","evidence_id":"evidence.trace.principle.managed-authoritative.v1","locator_role":"artifact","downstream_bead":"omni-test-acceptance-1"}"#,
    );
    assert!(report.valid, "findings: {:?}", report.findings);
}

#[test]
fn executable_locators_ignore_macro_token_trees_and_fail_closed_on_unbalanced_macros() {
    for (source, selector) in [
        (
            "macro_rules! fabricated { () => { fn exact_traceability_case() {} } }\n",
            "exact_traceability_case",
        ),
        (
            "macro_rules! fabricated { () => { mod nested { fn exact_traceability_case() {} } } }\n",
            "nested::exact_traceability_case",
        ),
        (
            "fabricated! { fn exact_traceability_case() {} }\n",
            "exact_traceability_case",
        ),
        (
            "fabricated!(mod nested { fn exact_traceability_case() {} })\n",
            "nested::exact_traceability_case",
        ),
        (
            "fabricated![fn exact_traceability_case() {}]\n",
            "exact_traceability_case",
        ),
        (
            "qualified::fabricated!({ mod nested { fn exact_traceability_case() {} } })\n",
            "nested::exact_traceability_case",
        ),
        (
            "mod nested { fabricated!({ [ (fn exact_traceability_case() {}) ] }) }\n",
            "nested::exact_traceability_case",
        ),
        (
            "#[fabricated({ fn exact_traceability_case() {} })]\nfn actual_traceability_case() {}\n",
            "exact_traceability_case",
        ),
        (
            "fabricated!({ [ (fn exact_traceability_case() {}) ] }\n",
            "exact_traceability_case",
        ),
        (
            "macro_rules! fabricated { () => { fn exact_traceability_case() {} }\n",
            "exact_traceability_case",
        ),
    ] {
        let (_directory, report) = file_backed_fixture(
            source,
            selector,
            r#"{"schema":"omnirepo.traceability-fixture.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","fixture_id":"fixture:managed-whole-and-partial","locator_role":"fixture","downstream_bead":"omni-test-acceptance-1"}"#,
            r#"{"schema":"omnirepo.traceability-evidence.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","evidence_id":"evidence.trace.principle.managed-authoritative.v1","locator_role":"artifact","downstream_bead":"omni-test-acceptance-1"}"#,
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "locator-unresolved"),
            "macro or malformed source must not resolve selector {selector:?} in {source:?}: {:?}",
            report.findings
        );
    }
}

#[test]
fn executable_locators_still_resolve_real_items_after_macro_token_trees() {
    let (_directory, report) = file_backed_fixture(
        "macro_rules! fabricated { () => { fn fake_traceability_case() {} } }\n#[fabricated(fn another_fake_traceability_case() {})]\nmod nested { fabricated!({ fn hidden_traceability_case() {} }) fn actual_traceability_case() {} }\n",
        "nested::actual_traceability_case",
        r#"{"schema":"omnirepo.traceability-fixture.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","fixture_id":"fixture:managed-whole-and-partial","locator_role":"fixture","downstream_bead":"omni-test-acceptance-1"}"#,
        r#"{"schema":"omnirepo.traceability-evidence.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","evidence_id":"evidence.trace.principle.managed-authoritative.v1","locator_role":"artifact","downstream_bead":"omni-test-acceptance-1"}"#,
    );
    assert!(
        report.valid,
        "real item after macros must resolve: {:?}",
        report.findings
    );
}

#[test]
fn executable_locators_ignore_comments_strings_and_prefixes() {
    for (source, selector) in [
        (
            "// fn exact_traceability_case() {}\n",
            "exact_traceability_case",
        ),
        (
            "/* outer /* nested fn exact_traceability_case() {} */ */\n",
            "exact_traceability_case",
        ),
        (
            "const TEXT: &str = \"fn exact_traceability_case() {}\";\n",
            "exact_traceability_case",
        ),
        (
            "const RAW: &str = r###\"fn exact_traceability_case() {}\"###;\n",
            "exact_traceability_case",
        ),
        (
            "fn exact_traceability_case_extra() {}\n",
            "exact_traceability_case",
        ),
        (
            "fn exact_traceability_case_suffix() {}\n",
            "exact_traceability_case",
        ),
    ] {
        let (_directory, report) = file_backed_fixture(
            source,
            selector,
            r#"{"schema":"omnirepo.traceability-fixture.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","fixture_id":"fixture:managed-whole-and-partial","locator_role":"fixture","downstream_bead":"omni-test-acceptance-1"}"#,
            r#"{"schema":"omnirepo.traceability-evidence.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","evidence_id":"evidence.trace.principle.managed-authoritative.v1","locator_role":"artifact","downstream_bead":"omni-test-acceptance-1"}"#,
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "locator-unresolved"),
            "hostile source must not resolve selector: {:?}",
            report.findings
        );
    }
}

#[test]
fn structured_fixture_and_evidence_records_bind_every_identity() {
    let (_directory, report) = file_backed_fixture(
        "fn exact_traceability_case() {}\n",
        "exact_traceability_case",
        r#"// {"schema":"omnirepo.traceability-fixture.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","fixture_id":"fixture:managed-whole-and-partial","locator_role":"fixture","downstream_bead":"omni-test-acceptance-1"}"#,
        r#"{"schema":"omnirepo.traceability-evidence.v1","row_id":"principle-managed-authoritative","case_id":"trace.principle.managed-authoritative","evidence_id":"wrong-evidence-id","locator_role":"artifact","downstream_bead":"omni-test-acceptance-1"}"#,
    );
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "locator-artifact-invalid")
    );
}

#[test]
fn verified_rows_reject_same_bead_and_generic_downstream_notes() {
    let same_bead = implemented_first_row(&matrix(), true, true).replace(
        "\"downstream_bead\":\"omni-test-acceptance-1\"",
        "\"downstream_bead\":\"omni-constitutional-convergence-2r9.64.1\"",
    );
    let report = traceability::validate_source(
        &same_bead,
        &append_synthetic_acceptance_bead(&beads(), true),
    );
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification-downstream-same")
    );

    let generic = implemented_first_row(&matrix(), true, true);
    let generic_beads = append_synthetic_acceptance_bead(&beads(), false);
    let report = traceability::validate_source(&generic, &generic_beads);
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification-evidence-provenance-missing")
    );

    let mismatched_beads = append_synthetic_acceptance_bead(&beads(), true).replace(
        "evidence.trace.principle.managed-authoritative.v1",
        "evidence.other.v1",
    );
    let report = traceability::validate_source(&generic, &mismatched_beads);
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification-evidence-provenance-missing")
    );
}

#[test]
fn constitutional_silence_must_be_an_explicit_boolean_on_every_row() {
    let missing = matrix().replacen(",\"constitutional_silence\":false", "", 1);
    let report = traceability::validate_source(&missing, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "silence-not-explicit")
    );

    let null = matrix().replacen(
        "\"constitutional_silence\":false",
        "\"constitutional_silence\":null",
        1,
    );
    let report = traceability::validate_source(&null, &beads());
    assert!(!report.valid);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "silence-not-explicit")
    );
}

#[test]
fn optional_and_silence_taxonomy_is_bidirectional() {
    let optional_test_type = matrix().replacen(
        "\"test_type\":\"black-box-e2e\"",
        "\"test_type\":\"optional\"",
        1,
    );
    let report = traceability::validate_source(&optional_test_type, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "silence-not-explicit")
    );

    let silence_effect = matrix().replacen(
        "\"expected_effect\":\"positive\"",
        "\"expected_effect\":\"silence\"",
        1,
    );
    let report = traceability::validate_source(&silence_effect, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "silence-not-explicit")
    );
}

#[test]
fn policy_assignment_detection_handles_spacing_and_selection_forms() {
    for assignment in [
        "policy = selected-value",
        "policy  :  selected_value",
        "selected value : owner-choice",
        "selection = explicit",
        "effective value : chosen",
        "override = selected",
    ] {
        let source = matrix().replacen(
            "A local edit inside a managed boundary is not preserved as competing authority.",
            assignment,
            1,
        );
        let report = traceability::validate_source(&source, &beads());
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "policy-value-forbidden"),
            "assignment form {assignment:?} was not rejected: {:?}",
            report.findings
        );
    }
}

#[test]
fn constitutional_and_adversarial_views_are_consumed_as_projections() {
    let broken_kind = matrix().replacen(
        "\"kind\":\"constitutional\",\"reference\":\"constitution:principle.1\"",
        "\"kind\":\"product-contract\",\"reference\":\"constitution:principle.1\"",
        1,
    );
    let report = traceability::validate_source(&broken_kind, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "projection-mismatch")
    );

    let broken_negative_projection = matrix().replacen(
        "\"negative_case\":\"A local edit inside a managed boundary is not preserved as competing authority.\"",
        "\"negative_case\":\"\"",
        1,
    );
    let report = traceability::validate_source(&broken_negative_projection, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "projection-missing-negative-case")
    );
}

#[test]
fn row_replay_identity_is_stable_when_an_unrelated_row_is_inserted() {
    let target = matrix().replacen(
        "\"id\":\"principle-managed-authoritative\"",
        "\"id\":\"invalid row id\"",
        1,
    );
    let original = traceability::validate_source(&target, &beads());
    let original_finding = original
        .findings
        .iter()
        .find(|finding| finding.path.ends_with(".id"))
        .expect("invalid row ID must produce an ID finding");
    let inserted = target.replacen(
        "    {\"id\":\"invalid row id\"",
        &format!(
            "    {},\n    {{\"id\":\"invalid row id\"",
            first_row(&matrix())
        ),
        1,
    );
    let inserted_report = traceability::validate_source(&inserted, &beads());
    let replayed = inserted_report
        .findings
        .iter()
        .find(|finding| finding.path.ends_with(".id"))
        .expect("inserted report must retain invalid row ID finding");
    assert_eq!(original_finding.replay_id, replayed.replay_id);
}

#[test]
fn policy_assignment_detection_is_broadened_without_selecting_values() {
    let source = matrix().replacen(
        "\"negative_case\":\"A local edit inside a managed boundary is not preserved as competing authority.\"",
        "\"negative_case\":\"policy: selected-value\"",
        1,
    );
    let report = traceability::validate_source(&source, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "policy-value-forbidden")
    );
}

#[test]
fn locators_reject_paths_outside_the_repository() {
    let source = implemented_first_row(&matrix(), false, false)
        .replace("tests/traceability_contract.rs", "../outside.rs");
    let report = traceability::validate_source(&source, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "locator-outside-repository")
    );
}

#[test]
fn oversized_and_unicode_inputs_are_bounded() {
    let oversized = format!("{{\"{}\":true}}", "x".repeat(1_048_577));
    let report = traceability::validate_source(&oversized, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "matrix-too-large")
    );

    let unicode_key = "é".repeat(300);
    let report = traceability::validate_source(&format!("{{\"{unicode_key}\":true}}"), &beads());
    assert!(!report.valid);
    for finding in report.findings {
        assert!(finding.path.len() <= 256);
        assert!(finding.message.len() <= 256);
        assert!(finding.path.is_char_boundary(finding.path.len()));
        assert!(finding.message.is_char_boundary(finding.message.len()));
    }
}

#[test]
fn nesting_depth_is_bounded() {
    let nested = format!("{{\"rows\":{}}}", "[".repeat(40) + &"]".repeat(40));
    let report = traceability::validate_source(&nested, &beads());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "schema-malformed")
    );
}

#[cfg(unix)]
#[test]
fn validate_file_rejects_external_and_symlink_inputs() {
    use std::os::unix::fs::symlink;

    // The external fixture must live OUTSIDE the repository root: the
    // suite runner pins TMPDIR under target/, so the system temp directory
    // can be inside the repo.  The repo's parent is deterministic and
    // outside the root.
    let external_dir =
        tempfile::tempdir_in(root().parent().expect("repo parent")).expect("external dir");
    let external = external_dir.path().join("external-matrix.json");
    std::fs::write(&external, "{}").expect("write external fixture");
    let external_error = traceability::validate_file(&external)
        .expect_err("external matrix must be rejected before reading");
    assert!(
        external_error
            .to_string()
            .contains("outside the repository")
    );

    let inside =
        tempfile::tempdir_in(root().join("tests/traceability")).expect("create fixture dir");
    let link = inside.path().join("matrix-link.json");
    symlink(&external, &link).expect("create hostile symlink fixture");
    let symlink_error = traceability::validate_file(&link)
        .expect_err("symlink matrix must be rejected before reading");
    assert!(symlink_error.to_string().contains("contains a symlink"));
}
