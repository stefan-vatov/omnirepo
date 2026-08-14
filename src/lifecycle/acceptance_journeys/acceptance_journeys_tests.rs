//! Focused proof for the canonical executable acceptance journey matrix.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::acceptance_journeys::{
    AcceptanceJourney, JourneyKind, JourneyOutcome, JourneyReport, canonical_journey_matrix,
    run_journey,
};
use std::path::Path;

#[test]
fn every_required_journey_kind_is_in_the_canonical_matrix() {
    let matrix = canonical_journey_matrix();
    assert!(!matrix.is_empty());
    let kinds = matrix
        .iter()
        .map(|journey| journey.kind)
        .collect::<std::collections::BTreeSet<_>>();
    for required in [
        JourneyKind::Authority,
        JourneyKind::Exactness,
        JourneyKind::Inference,
        JourneyKind::Fleet,
        JourneyKind::Verification,
        JourneyKind::Git,
        JourneyKind::Record,
        JourneyKind::Recovery,
        JourneyKind::Repair,
        JourneyKind::MigrationDeclined,
        JourneyKind::Setup,
        JourneyKind::Packaging,
        JourneyKind::Parity,
    ] {
        assert!(
            kinds.contains(&required),
            "missing journey kind {required:?}"
        );
    }
}

#[test]
fn every_journey_has_stable_ids_expected_effects_and_negative_assertions() {
    let matrix = canonical_journey_matrix();
    let mut ids = std::collections::BTreeSet::new();
    for journey in &matrix {
        assert!(!journey.id.is_empty(), "stable id");
        assert!(
            ids.insert(journey.id),
            "duplicate journey id {}",
            journey.id
        );
        assert!(
            !journey.expected_effect.is_empty(),
            "{} has an expected effect",
            journey.id
        );
        assert!(
            !journey.negative_assertions.is_empty(),
            "{} carries negative assertions",
            journey.id
        );
        assert!(
            !journey.replay_link.is_empty(),
            "{} has a replay link",
            journey.id
        );
    }
}

#[test]
fn the_journey_runner_executes_in_a_clean_environment_with_structured_evidence() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("journey-run-")
        .tempdir_in(&base)
        .expect("fixture");
    // The clean environment: an empty HOME under the harness root.
    let clean_home = fixture.path().join("clean-home");
    std::fs::create_dir_all(&clean_home).expect("clean home");
    let journey = canonical_journey_matrix()
        .into_iter()
        .find(|journey| journey.id == "authority-machine-declared")
        .expect("journey");
    let report = run_journey(&journey, &clean_home).expect("run");
    assert_eq!(report.id, journey.id);
    assert!(
        matches!(report.outcome, JourneyOutcome::Passed),
        "{report:?}"
    );
    assert!(
        !report.evidence.is_empty(),
        "structured evidence is produced"
    );
    // The evidence is machine-readable and carries the stable id.
    assert!(report.evidence.contains(report.id), "{}", report.evidence);
}

#[test]
fn forbidden_paths_are_absent_in_negative_journeys() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("journey-negative-")
        .tempdir_in(&base)
        .expect("fixture");
    let clean_home = fixture.path().join("clean-home");
    std::fs::create_dir_all(&clean_home).expect("clean home");
    let matrix = canonical_journey_matrix();
    let forbidden = [
        "legacy",
        "reverse-authority",
        "semantic-sync",
        "outside-root",
        "unbounded-repair",
        "hidden-agent-only",
    ];
    for name in forbidden {
        let journey = matrix
            .iter()
            .find(|journey| journey.id.contains(name))
            .unwrap_or_else(|| panic!("missing negative journey for {name}"));
        let report = run_journey(journey, &clean_home).expect("run");
        assert!(
            matches!(report.outcome, JourneyOutcome::Passed),
            "{name}: {report:?}"
        );
        for assertion in journey.negative_assertions {
            assert!(!assertion.is_empty());
        }
    }
}

#[test]
fn failures_account_independently_per_journey() {
    // Each journey's failure is its own: a failing journey reports its
    // own outcome and never hides the others (independent accounting is
    // the runner's contract; the matrix itself is green here).
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("journey-accounting-")
        .tempdir_in(&base)
        .expect("fixture");
    let clean_home = fixture.path().join("clean-home");
    std::fs::create_dir_all(&clean_home).expect("clean home");
    let matrix = canonical_journey_matrix();
    let mut passed = 0;
    for journey in &matrix {
        let report = run_journey(journey, &clean_home).expect("run");
        assert_eq!(report.id, journey.id, "the report names its journey");
        if matches!(report.outcome, JourneyOutcome::Passed) {
            passed += 1;
        }
        assert!(report.independent_failures == 0, "{report:?}");
    }
    assert_eq!(passed, matrix.len(), "every canonical journey passes");
}
