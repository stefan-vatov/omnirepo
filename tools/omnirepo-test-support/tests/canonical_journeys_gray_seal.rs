//! The one executable acceptance-journey matrix for the constitutional suite.
//!
//! The traceability matrix is the authority for stable row, case, evidence,
//! replay, owner, and status identities.  This target projects those rows
//! into reportable groups.  It does not copy a second matrix or select product
//! policy.  Product rows that are still `specified` are recorded as structured
//! skips until their implementation Beads land; a skip is not a product pass.

use std::{collections::BTreeSet, path::PathBuf, process::Command};

use omnirepo_test_support::{
    e2e_runner_crimson_coast::{
        E2eRunner, ExpectedEffects, ExpectedFile, FixtureBinarySpec, RunReport, RunnerCase,
    },
    failure_replay::ReplayCommand,
    test_evidence::{
        ArtifactReference, DiagnosticRedactor, EventKind, EventRecorder, EvidenceBundle, Outcome,
        SourcePlanConfig, TestIdentity,
    },
};
use serde_json::Value;

const CANONICAL_TRACEABILITY_MATRIX: &[u8] =
    include_bytes!("../../../tests/traceability/matrix.json");
const MATRIX_SCHEMA: &str = "omnirepo.traceability-matrix.v1";
const MATRIX_STATUS: &str = "canonical";
const EVIDENCE_SCHEMA: &str = "omnirepo.test-evidence-bundle.v1";
const SUITE: &str = "canonical-acceptance-journeys";
const SEED_SALT: u64 = 74_700;
const MIGRATION_DECISION_BEAD: &str = "omni-constitutional-convergence-2r9.26";
const MIGRATION_IMPLEMENTATION_BEAD: &str = "omni-constitutional-convergence-2r9.13";
const REPLAY_CASE_ENV: &str = "OMNIREPO_CANONICAL_REPLAY_CASE";
const REPLAY_DISPATCH_TEST: &str = "canonical_journey_replay_dispatch";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum JourneyGroup {
    Authority,
    Exactness,
    Inference,
    Precedence,
    FleetLifecycle,
    RepairParity,
    SetupPackaging,
    Optional,
}

impl JourneyGroup {
    const ALL: [Self; 8] = [
        Self::Authority,
        Self::Exactness,
        Self::Inference,
        Self::Precedence,
        Self::FleetLifecycle,
        Self::RepairParity,
        Self::SetupPackaging,
        Self::Optional,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Authority => "authority",
            Self::Exactness => "exactness",
            Self::Inference => "inference",
            Self::Precedence => "precedence",
            Self::FleetLifecycle => "fleet-lifecycle",
            Self::RepairParity => "repair-parity",
            Self::SetupPackaging => "setup-packaging",
            Self::Optional => "optional",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PublicInvocation {
    human_argv: Vec<String>,
    agent_argv: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeclaredEffects {
    expected_effect: String,
    expected_files_or_sections: String,
    expected_git_refs: String,
    expected_records: String,
    expected_stdout: String,
    expected_stderr: String,
    expected_exit_status: String,
    forbidden_effects: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JourneyCase {
    row_id: String,
    reference: String,
    case_id: String,
    fixture_id: String,
    evidence_id: String,
    replay_id: String,
    primary_owner: String,
    implementation_bead: String,
    downstream_bead: String,
    implementation_status: String,
    verification_status: String,
    test_locator_role: String,
    evidence_locator_role: String,
    owner_decision_refs: Vec<String>,
    constitutional_silence: bool,
    expected_observation: String,
    groups: BTreeSet<JourneyGroup>,
    effects: DeclaredEffects,
    invocation: PublicInvocation,
    replay_recipe: ReplayCommand,
}

fn required_string(row: &Value, field: &str) -> String {
    row.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("canonical row is missing non-empty {field}"))
}

fn required_bool(row: &Value, field: &str) -> bool {
    row.get(field)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("canonical row is missing boolean {field}"))
}

fn required_strings(row: &Value, field: &str) -> Vec<String> {
    row.get(field)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("canonical row is missing array {field}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| panic!("canonical row {field} contains a non-string value"))
        })
        .collect()
}

fn locator_role(row: &Value, field: &str) -> String {
    row.get(field)
        .and_then(Value::as_object)
        .and_then(|locator| locator.get("role"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("canonical row is missing {field}.role"))
}

fn stable_seed(case_id: &str) -> u64 {
    case_id.bytes().fold(SEED_SALT, |seed, byte| {
        seed.wrapping_mul(131).wrapping_add(u64::from(byte))
    })
}

fn groups_for(reference: &str) -> BTreeSet<JourneyGroup> {
    let mut groups = BTreeSet::new();
    let add = |groups: &mut BTreeSet<JourneyGroup>, group| {
        groups.insert(group);
    };

    match reference {
        "constitution:principle.1" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Exactness);
        }
        "constitution:principle.2" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Inference);
        }
        "constitution:principle.3" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Exactness);
        }
        "constitution:principle.4" => {
            add(&mut groups, JourneyGroup::Exactness);
        }
        "constitution:principle.5" => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
        }
        "constitution:principle.6" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Precedence);
        }
        "constitution:principle.7" => {
            add(&mut groups, JourneyGroup::RepairParity);
        }
        "constitution:principle.8" => {
            add(&mut groups, JourneyGroup::SetupPackaging);
        }
        "constitution:growth-directive.1" => {
            add(&mut groups, JourneyGroup::Inference);
            add(&mut groups, JourneyGroup::SetupPackaging);
        }
        "constitution:growth-directive.2" => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
        }
        "constitution:growth-directive.3" => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
            add(&mut groups, JourneyGroup::RepairParity);
        }
        "constitution:growth-directive.4" => {
            add(&mut groups, JourneyGroup::Exactness);
        }
        "constitution:growth-directive.5" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::SetupPackaging);
        }
        "constitution:boundary.1" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::FleetLifecycle);
        }
        "constitution:boundary.2" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Precedence);
        }
        "constitution:boundary.3" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Inference);
        }
        "constitution:boundary.4" => {
            add(&mut groups, JourneyGroup::Exactness);
        }
        "constitution:boundary.5" => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
            add(&mut groups, JourneyGroup::RepairParity);
        }
        "constitution:tension.1" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Precedence);
        }
        "constitution:tension.2" => {
            add(&mut groups, JourneyGroup::Inference);
            add(&mut groups, JourneyGroup::Precedence);
        }
        "constitution:tension.3" | "constitution:tension.4" | "constitution:tension.5" => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
        }
        "constitution:tension.6" => {
            add(&mut groups, JourneyGroup::Exactness);
        }
        "optional:model-based-testing" => {
            add(&mut groups, JourneyGroup::Optional);
        }
        _ if reference.starts_with("command:") => {
            if reference == "command:sync" {
                add(&mut groups, JourneyGroup::FleetLifecycle);
                add(&mut groups, JourneyGroup::RepairParity);
            } else {
                add(&mut groups, JourneyGroup::SetupPackaging);
            }
        }
        _ if reference.starts_with("failure:") => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
            if matches!(
                reference,
                "failure:repair" | "failure:cancellation-recovery"
            ) {
                add(&mut groups, JourneyGroup::RepairParity);
            }
        }
        "behavior:configuration-authority" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Inference);
        }
        "behavior:source-materialization" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Precedence);
        }
        "behavior:repository-policy" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Inference);
        }
        "behavior:whole-file-sync" | "behavior:partial-section-sync" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Exactness);
        }
        "behavior:containment" => {
            add(&mut groups, JourneyGroup::Authority);
            add(&mut groups, JourneyGroup::Exactness);
        }
        "behavior:fleet-progress"
        | "behavior:verification"
        | "behavior:git-delivery"
        | "behavior:run-record" => {
            add(&mut groups, JourneyGroup::FleetLifecycle);
        }
        "behavior:repair-causation" => {
            add(&mut groups, JourneyGroup::RepairParity);
        }
        "behavior:setup" | "behavior:validate" | "behavior:packaging" => {
            add(&mut groups, JourneyGroup::SetupPackaging);
        }
        _ => panic!("canonical reference has no journey group: {reference}"),
    }
    groups
}

fn invocation_for(reference: &str) -> PublicInvocation {
    let command = if matches!(reference, "command:setup" | "behavior:setup") {
        "setup"
    } else if matches!(reference, "command:validate" | "behavior:validate") {
        "validate"
    } else {
        "sync"
    };
    PublicInvocation {
        human_argv: vec![command.to_owned()],
        agent_argv: vec![command.to_owned()],
    }
}

fn replay_recipe(case_id: &str) -> ReplayCommand {
    ReplayCommand::new(
        "env",
        [
            format!("{REPLAY_CASE_ENV}={case_id}"),
            "cargo".to_owned(),
            "test".to_owned(),
            "-p".to_owned(),
            "omnirepo-test-support".to_owned(),
            "--test".to_owned(),
            "canonical_journeys_gray_seal".to_owned(),
            "--locked".to_owned(),
            "--".to_owned(),
            "--exact".to_owned(),
            REPLAY_DISPATCH_TEST.to_owned(),
            "--nocapture".to_owned(),
            "--test-threads=1".to_owned(),
        ],
    )
}

fn is_planned(case: &JourneyCase) -> bool {
    case.implementation_status == "specified" && case.test_locator_role == "planned"
}

fn public_product_binary() -> Result<PathBuf, String> {
    let path = std::env::var_os("CARGO_BIN_EXE_omnirepo")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "executable journey requires Cargo's CARGO_BIN_EXE_omnirepo public binary path"
                .to_owned()
        })?;
    if !path.is_absolute() || !path.is_file() {
        return Err(format!(
            "CARGO_BIN_EXE_omnirepo is not an existing absolute executable: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn declared_effects(
    fixture_id: &str,
    expected_effect: &str,
    expected_observation: &str,
    negative_case: &str,
    implementation_status: &str,
    verification_status: &str,
) -> DeclaredEffects {
    let pending = if implementation_status == "specified" {
        format!(
            "not-observed: implementation_status={implementation_status}; verification_status={verification_status}"
        )
    } else {
        expected_observation.to_owned()
    };
    DeclaredEffects {
        expected_effect: expected_effect.to_owned(),
        expected_files_or_sections: format!(
            "{pending}; fixture={fixture_id}; contract={expected_observation}"
        ),
        expected_git_refs: pending.clone(),
        expected_records: pending.clone(),
        expected_stdout: pending.clone(),
        expected_stderr: pending.clone(),
        expected_exit_status: pending,
        forbidden_effects: vec![negative_case.to_owned()],
    }
}

fn journey_case(row: &Value) -> JourneyCase {
    let reference = required_string(row, "reference");
    let case_id = required_string(row, "case_id");
    let fixture_id = required_string(row, "fixture");
    let expected_effect = required_string(row, "expected_effect");
    let expected_observation = required_string(row, "expected_observation");
    let negative_case = required_string(row, "negative_case");
    let implementation_status = required_string(row, "implementation_status");
    let verification_status = required_string(row, "verification_status");
    let groups = groups_for(&reference);
    assert!(
        !groups.is_empty(),
        "journey must belong to one group: {case_id}"
    );
    JourneyCase {
        row_id: required_string(row, "id"),
        reference: reference.clone(),
        case_id: case_id.clone(),
        fixture_id: fixture_id.clone(),
        evidence_id: required_string(row, "evidence_id"),
        replay_id: required_string(row, "replay_id"),
        primary_owner: required_string(row, "primary_owner"),
        implementation_bead: required_string(row, "implementation_bead"),
        downstream_bead: required_string(row, "downstream_bead"),
        implementation_status: implementation_status.clone(),
        verification_status: verification_status.clone(),
        test_locator_role: locator_role(row, "test_locator"),
        evidence_locator_role: locator_role(row, "evidence_locator"),
        owner_decision_refs: required_strings(row, "owner_decision_refs"),
        constitutional_silence: required_bool(row, "constitutional_silence"),
        expected_observation: expected_observation.clone(),
        groups,
        effects: declared_effects(
            &fixture_id,
            &expected_effect,
            &expected_observation,
            &negative_case,
            &implementation_status,
            &verification_status,
        ),
        invocation: invocation_for(&reference),
        replay_recipe: replay_recipe(&case_id),
    }
}

fn canonical_cases() -> Vec<JourneyCase> {
    let matrix: Value = serde_json::from_slice(CANONICAL_TRACEABILITY_MATRIX)
        .expect("canonical traceability matrix should be valid JSON");
    assert_eq!(
        matrix.get("schema").and_then(Value::as_str),
        Some(MATRIX_SCHEMA)
    );
    assert_eq!(
        matrix.get("status").and_then(Value::as_str),
        Some(MATRIX_STATUS)
    );
    matrix
        .get("rows")
        .and_then(Value::as_array)
        .expect("canonical traceability matrix should have rows")
        .iter()
        .map(journey_case)
        .collect()
}

fn case_identity(case: &JourneyCase) -> TestIdentity {
    TestIdentity::new(
        &case.case_id,
        SUITE,
        "canonical-destination",
        "acceptance",
        SourcePlanConfig::new(
            "traceability-matrix.v1",
            &case.case_id,
            "owner-decision-snapshot",
        )
        .expect("canonical source/plan/config identity should be valid"),
        1,
        stable_seed(&case.case_id),
        "black-box-e2e",
    )
    .expect("canonical journey identity should be valid")
}

fn case_artifact(case: &JourneyCase) -> ArtifactReference {
    ArtifactReference::new(
        PathBuf::from(format!("journeys/{}/evidence.jsonl", case.case_id)),
        case.replay_id.clone(),
    )
    .expect("canonical journey artifact reference should be safe")
}

fn pending_stage_diagnostic(case: &JourneyCase) -> String {
    format!(
        "not-executable: traceability_status={}; implementation_bead={}; verification_status={}; primary_owner={}; fixture={}; expected_effect={}; expected_observation={}; expected_files_or_sections={}; expected_git_refs={}; expected_records={}; expected_stdout={}; expected_stderr={}; expected_exit_status={}; forbidden_effects={:?}; owner_decision_refs={:?}; evidence_id={}; replay_id={}; downstream_bead={}",
        case.implementation_status,
        case.implementation_bead,
        case.verification_status,
        case.primary_owner,
        case.fixture_id,
        case.effects.expected_effect,
        case.expected_observation,
        case.effects.expected_files_or_sections,
        case.effects.expected_git_refs,
        case.effects.expected_records,
        case.effects.expected_stdout,
        case.effects.expected_stderr,
        case.effects.expected_exit_status,
        case.effects.forbidden_effects,
        case.owner_decision_refs,
        case.evidence_id,
        case.replay_id,
        case.downstream_bead,
    )
}

enum JourneyExecution {
    Pending(String),
    Passed(Box<RunReport>),
}

fn executable_expected_effects(case: &JourneyCase) -> ExpectedEffects {
    let expected = ExpectedEffects::success();
    if case.effects.expected_effect == "silence" {
        expected.exact_files(std::iter::empty::<ExpectedFile>())
    } else {
        expected
    }
}

fn validate_executable_report(case: &JourneyCase, report: &RunReport) -> Result<(), String> {
    if report.case_id != case.case_id {
        return Err(format!(
            "public binary returned evidence for {} while journey selected {}",
            report.case_id, case.case_id
        ));
    }
    if !report.process.success() {
        return Err(format!(
            "public binary did not satisfy expected exit contract for {}: {:?}",
            case.case_id, report.process.code
        ));
    }
    if !report.containment.no_outside_writes() {
        return Err(format!(
            "forbidden outside or unauthorized effect observed for {}",
            case.case_id
        ));
    }
    if report.git.unexpected_changes {
        return Err(format!(
            "forbidden unexpected Git/ref effect observed for {}",
            case.case_id
        ));
    }
    if report.evidence_bundle.projection.outcome != Outcome::Passed {
        return Err(format!(
            "public binary evidence did not pass for {}: {:?}",
            case.case_id, report.evidence_bundle.projection.outcome
        ));
    }

    let declared = [
        (
            "expected_files_or_sections",
            case.effects.expected_files_or_sections.as_str(),
        ),
        ("expected_git_refs", case.effects.expected_git_refs.as_str()),
        ("expected_records", case.effects.expected_records.as_str()),
        ("expected_stdout", case.effects.expected_stdout.as_str()),
        ("expected_stderr", case.effects.expected_stderr.as_str()),
        (
            "expected_exit_status",
            case.effects.expected_exit_status.as_str(),
        ),
    ];
    for (field, value) in declared {
        if value.is_empty() || value.starts_with("not-observed:") {
            return Err(format!(
                "executable journey {} has no concrete {field} contract",
                case.case_id
            ));
        }
    }
    if case.effects.forbidden_effects.is_empty()
        || case.effects.forbidden_effects.iter().any(String::is_empty)
    {
        return Err(format!(
            "executable journey {} has no forbidden-effect contract",
            case.case_id
        ));
    }
    Ok(())
}

fn run_executable_case(case: &JourneyCase) -> Result<RunReport, String> {
    let binary = public_product_binary()?;
    let runner_case = RunnerCase::new(
        &case.case_id,
        FixtureBinarySpec::existing("omnirepo", binary),
    )
    .map_err(|error| error.to_string())?
    .seed(stable_seed(&case.case_id))
    .args(case.invocation.human_argv.clone())
    .map_err(|error| error.to_string())?
    .expected(executable_expected_effects(case));
    let report = E2eRunner::new()
        .run(runner_case)
        .map_err(|error| error.to_string())?;
    validate_executable_report(case, &report)?;
    Ok(report)
}

fn execute_case(case: &JourneyCase) -> Result<JourneyExecution, String> {
    if is_planned(case) {
        return Ok(JourneyExecution::Pending(pending_stage_diagnostic(case)));
    }
    if case.implementation_status == "implemented" && case.test_locator_role == "executable" {
        return run_executable_case(case).map(|report| JourneyExecution::Passed(Box::new(report)));
    }
    Err(format!(
        "traceability row {} has an unsupported execution status: implementation_status={:?}, test_locator_role={:?}",
        case.case_id, case.implementation_status, case.test_locator_role
    ))
}

fn record_group(cases: &[JourneyCase], group: Option<JourneyGroup>) -> EvidenceBundle {
    assert!(
        !cases.is_empty(),
        "filtered journey group must not be empty"
    );
    let ordered_case_ids = cases
        .iter()
        .map(|case| case.case_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let recorder = EventRecorder::new(DiagnosticRedactor::default());
    for case in cases {
        recorder
            .expect(case_identity(case))
            .expect("each canonical journey must register as a peer");
    }
    let mut failed_case_ids = BTreeSet::new();
    let mut execution_failures = Vec::new();
    for case in cases {
        let guard = recorder
            .start(case_identity(case), case_artifact(case))
            .expect("each canonical journey must start an evidence step");
        match execute_case(case) {
            Ok(JourneyExecution::Pending(diagnostic)) => guard
                .skip(diagnostic)
                .expect("pending journey must terminalize as skipped"),
            Ok(JourneyExecution::Passed(report)) => {
                assert_eq!(report.evidence_bundle.schema, EVIDENCE_SCHEMA);
                guard
                    .pass()
                    .expect("executable journey must terminalize as passed");
            }
            Err(error) => {
                failed_case_ids.insert(case.case_id.clone());
                execution_failures.push(format!("{}: {error}", case.case_id));
                guard
                    .fail(format!("executable journey blocked or failed: {error}"))
                    .expect("failed journey must terminalize as failed");
            }
        }
    }
    let bundle = recorder
        .finalize()
        .expect("canonical journey peer evidence must finalize");
    assert_eq!(bundle.schema, EVIDENCE_SCHEMA);
    assert_eq!(bundle.peer_accounting.expected_case_ids, ordered_case_ids);
    assert_eq!(bundle.peer_accounting.terminal_case_ids, ordered_case_ids);
    assert!(bundle.peer_accounting.missing_case_ids.is_empty());
    let planned_count = cases.iter().filter(|case| is_planned(case)).count();
    assert_eq!(bundle.projection.skipped, planned_count);
    assert_eq!(bundle.projection.failed, failed_case_ids.len());
    assert_eq!(
        bundle.projection.passed,
        cases.len() - planned_count - failed_case_ids.len()
    );
    assert_eq!(bundle.projection.harness_failures, 0);
    let expected_outcome = if !failed_case_ids.is_empty() {
        Outcome::Failed
    } else if planned_count == cases.len() {
        Outcome::Skipped
    } else {
        Outcome::Passed
    };
    assert_eq!(bundle.projection.outcome, expected_outcome);
    for event in &bundle.events {
        if event.event_kind != EventKind::Terminal {
            continue;
        }
        let case = cases
            .iter()
            .find(|case| case.case_id == event.identity.case_id)
            .expect("terminal evidence must link to a canonical case");
        let expected_event_outcome = if is_planned(case) {
            Outcome::Skipped
        } else if failed_case_ids.contains(&case.case_id) {
            Outcome::Failed
        } else {
            Outcome::Passed
        };
        assert_eq!(event.outcome, expected_event_outcome);
        assert_eq!(
            event.artifact.replay_id.as_deref(),
            Some(case.replay_id.as_str())
        );
        assert_eq!(
            event.artifact.path.as_deref(),
            Some(format!("journeys/{}/evidence.jsonl", case.case_id).as_str())
        );
        if is_planned(case) || failed_case_ids.contains(&case.case_id) {
            let diagnostic = event
                .diagnostic
                .as_deref()
                .expect("non-passing case must retain a structured diagnostic");
            assert!(diagnostic.contains(case.case_id.as_str()));
            assert!(diagnostic.contains(case.effects.expected_effect.as_str()));
            assert!(diagnostic.contains(case.effects.forbidden_effects[0].as_str()));
        } else {
            assert!(event.diagnostic.is_none());
        }
    }
    if let Some(group) = group {
        assert!(
            cases.iter().all(|case| case.groups.contains(&group)),
            "filtered evidence includes a case outside group {}",
            group.as_str()
        );
    }
    let serialized = bundle
        .to_jsonl()
        .expect("canonical evidence should serialize");
    assert_eq!(
        EvidenceBundle::from_jsonl(&serialized).expect("canonical evidence should replay"),
        bundle
    );
    assert!(
        execution_failures.is_empty(),
        "canonical executable journey failures: {execution_failures:?}"
    );
    bundle
}

fn cases_for_group(cases: &[JourneyCase], group: JourneyGroup) -> Vec<JourneyCase> {
    cases
        .iter()
        .filter(|case| case.groups.contains(&group))
        .cloned()
        .collect()
}

fn assert_group_projection(cases: &[JourneyCase], group: JourneyGroup) {
    let selected = cases_for_group(cases, group);
    let first = record_group(&selected, Some(group));
    let second = record_group(&selected, Some(group));
    assert_eq!(
        first
            .to_jsonl()
            .expect("first group evidence should serialize"),
        second
            .to_jsonl()
            .expect("second group evidence should serialize"),
        "group {} must be deterministic",
        group.as_str()
    );
}

#[test]
fn canonical_matrix_projects_every_row_with_stable_identity_and_truthful_status() {
    let cases = canonical_cases();
    assert_eq!(cases.len(), 57);

    let mut rows = BTreeSet::new();
    let mut case_ids = BTreeSet::new();
    let mut evidence_ids = BTreeSet::new();
    let mut replay_ids = BTreeSet::new();
    for case in &cases {
        assert!(
            rows.insert(case.row_id.clone()),
            "duplicate row ID {}",
            case.row_id
        );
        assert!(
            case_ids.insert(case.case_id.clone()),
            "duplicate case ID {}",
            case.case_id
        );
        assert!(
            evidence_ids.insert(case.evidence_id.clone()),
            "duplicate evidence ID {}",
            case.evidence_id
        );
        assert!(
            replay_ids.insert(case.replay_id.clone()),
            "duplicate replay ID {}",
            case.replay_id
        );
        assert!(
            matches!(
                case.effects.expected_effect.as_str(),
                "positive" | "negative" | "conditional" | "silence"
            ),
            "unsupported expected effect for {}",
            case.case_id
        );
        assert!(!case.effects.forbidden_effects.is_empty());
        assert!(!case.groups.is_empty());
        assert_eq!(case.replay_id, format!("replay.{}.v1", case.case_id));
        assert_eq!(case.evidence_id, format!("evidence.{}.v1", case.case_id));
        assert_eq!(case.replay_recipe.program(), "env");
        assert_eq!(
            case.replay_recipe.args().first(),
            Some(&format!("{REPLAY_CASE_ENV}={}", case.case_id))
        );
        assert!(
            case.replay_recipe
                .args()
                .windows(2)
                .any(|args| args == ["--exact", REPLAY_DISPATCH_TEST])
        );
        assert_eq!(case.replay_recipe.recipe(), case.replay_recipe.render());
        assert_eq!(case.invocation.human_argv, case.invocation.agent_argv);
        assert!(
            case.invocation.human_argv.iter().all(|argument| !matches!(
                argument.as_str(),
                "--agent" | "--internal" | "--authority"
            )),
            "agent-only authority flag found in {}",
            case.case_id
        );
        match case.implementation_status.as_str() {
            "specified" => {
                assert_eq!(case.verification_status, "specified");
                assert_eq!(case.test_locator_role, "planned");
                assert_eq!(case.evidence_locator_role, "planned");
            }
            "implemented" => assert_eq!(case.test_locator_role, "executable"),
            other => panic!("unsupported implementation status {other:?}"),
        }
        assert_eq!(
            case.constitutional_silence,
            case.effects.expected_effect == "silence"
        );
        if case.constitutional_silence {
            assert!(case.groups.contains(&JourneyGroup::Optional));
        }
    }
    for group in JourneyGroup::ALL {
        assert!(
            cases.iter().any(|case| case.groups.contains(&group)),
            "canonical matrix has no {} group projection",
            group.as_str()
        );
    }
}

fn replay_selected_case() -> Option<JourneyCase> {
    let requested = match std::env::var(REPLAY_CASE_ENV) {
        Ok(requested) => requested,
        Err(_) => return None,
    };
    let selected = canonical_cases()
        .into_iter()
        .filter(|case| case.case_id == requested)
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        1,
        "replay selection must resolve exactly one canonical row for {requested:?}"
    );
    Some(
        selected
            .into_iter()
            .next()
            .expect("exactly one replay row should be present"),
    )
}

#[test]
fn canonical_journey_replay_dispatch() {
    let Some(case) = replay_selected_case() else {
        // The normal matrix run does not select a replay row. The replay
        // recipe always supplies the variable and is checked for one test by
        // replay_recipe_executes_one_selected_row.
        return;
    };
    match execute_case(&case) {
        Ok(JourneyExecution::Pending(diagnostic)) => {
            assert!(diagnostic.contains(case.case_id.as_str()));
            assert!(diagnostic.contains(case.implementation_bead.as_str()));
            assert!(diagnostic.contains(case.replay_id.as_str()));
        }
        Ok(JourneyExecution::Passed(report)) => {
            assert_eq!(report.case_id, case.case_id);
            assert_eq!(report.evidence_bundle.projection.outcome, Outcome::Passed);
        }
        Err(error) => panic!("selected replay row failed closed: {error}"),
    }
}

#[test]
fn replay_recipe_executes_one_selected_row() {
    let case = canonical_cases()
        .into_iter()
        .find(|case| case.case_id == "trace.principle.managed-authoritative")
        .expect("replay proof row should remain in the canonical matrix");
    let recipe = case.replay_recipe;
    let output = Command::new(recipe.program())
        .args(recipe.args())
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .expect("canonical replay recipe should spawn");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "canonical replay recipe failed: stdout={stdout}; stderr={stderr}"
    );
    let running_lines = stdout
        .lines()
        .filter(|line| line.starts_with("running "))
        .collect::<Vec<_>>();
    assert_eq!(running_lines, ["running 1 test"]);
    let selected_tests = stdout
        .lines()
        .filter(|line| line.starts_with("test ") && !line.starts_with("test result"))
        .collect::<Vec<_>>();
    assert_eq!(selected_tests.len(), 1);
    assert_eq!(
        selected_tests[0],
        format!("test {REPLAY_DISPATCH_TEST} ... ok")
    );
    assert!(stdout.contains("test result: ok. 1 passed; 0 failed;"));
    assert!(!stdout.contains("running 0 tests"));
}

#[test]
fn authority_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::Authority);
}

#[test]
fn exactness_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::Exactness);
}

#[test]
fn inference_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::Inference);
}

#[test]
fn precedence_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::Precedence);
}

#[test]
fn fleet_lifecycle_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::FleetLifecycle);
}

#[test]
fn repair_parity_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::RepairParity);
}

#[test]
fn setup_packaging_group_is_a_filtered_view_with_complete_peer_accounting() {
    assert_group_projection(&canonical_cases(), JourneyGroup::SetupPackaging);
}

#[test]
fn optional_group_is_silent_and_owner_decision_gated() {
    let cases = canonical_cases();
    let optional = cases_for_group(&cases, JourneyGroup::Optional);
    assert_eq!(optional.len(), 1);
    assert_eq!(optional[0].reference, "optional:model-based-testing");
    assert!(optional[0].constitutional_silence);
    assert_eq!(optional[0].effects.expected_effect, "silence");
    record_group(&optional, Some(JourneyGroup::Optional));
}

#[test]
fn complete_matrix_is_one_projection_and_retains_every_peer_result() {
    let cases = canonical_cases();
    let evidence = record_group(&cases, None);
    assert_eq!(
        evidence.peer_accounting.expected_case_ids.len(),
        cases.len()
    );
    assert_eq!(
        evidence.peer_accounting.terminal_case_ids.len(),
        cases.len()
    );
    assert!(evidence.peer_accounting.missing_case_ids.is_empty());
    assert!(!evidence.events.is_empty());
}

#[test]
fn optional_migration_has_no_implicit_public_path_without_owner_selection() {
    let cases = canonical_cases();
    let evolution = cases
        .iter()
        .find(|case| case.reference == "constitution:principle.8")
        .expect("evolution row should be present");
    assert!(
        evolution
            .owner_decision_refs
            .iter()
            .any(|reference| reference == MIGRATION_DECISION_BEAD)
    );

    // The approved owner decision declines an initial automated migration.
    // Keep this gate explicit: if a later owner decision selects migration,
    // that decision must add a separate implementation and evidence path.
    let selected = false;
    assert!(!selected);
    assert_eq!(
        MIGRATION_IMPLEMENTATION_BEAD,
        "omni-constitutional-convergence-2r9.13"
    );
    assert!(cases.iter().all(|case| {
        !case
            .invocation
            .human_argv
            .iter()
            .any(|argument| argument == "migrate")
    }));
}

#[test]
fn runner_stage_probe_uses_public_e2e_effect_and_evidence_contracts() {
    // This probe proves only the shared runner seam.  It is deliberately not
    // product acceptance: all product rows above remain structured skips while
    // their implementation Beads are specified rather than landed.
    let case = RunnerCase::new(
        "canonical-stage-probe",
        FixtureBinarySpec::shell(
            "stage-probe",
            "#!/bin/sh\nset -eu\nprintf 'traceability-status=specified\\n' > \"$OMNIREPO_E2E_EFFECTS_ROOT/stage.txt\"\n",
        ),
    )
    .expect("stage probe case should have a stable ID")
    .seed(SEED_SALT)
    .expected(
        ExpectedEffects::success().exact_files([ExpectedFile::with_contents(
            "stage.txt",
            b"traceability-status=specified\n".to_vec(),
        )]),
    );
    let report = E2eRunner::new()
        .run(case)
        .expect("runner stage probe should pass");
    assert_eq!(report.process.code, Some(0));
    assert!(report.containment.no_outside_writes());
    assert_eq!(report.evidence_bundle.projection.outcome, Outcome::Passed);
    assert_eq!(report.evidence_bundle.schema, EVIDENCE_SCHEMA);
    let replayed = EvidenceBundle::from_jsonl(&report.evidence_json)
        .expect("runner evidence should be replayable");
    assert_eq!(replayed, report.evidence_bundle);
    assert!(!report.replay_id.is_empty());
    assert!(
        report.evidence_bundle.events.iter().all(|event| {
            event.artifact.replay_id.as_deref() == Some(report.replay_id.as_str())
        })
    );
}
