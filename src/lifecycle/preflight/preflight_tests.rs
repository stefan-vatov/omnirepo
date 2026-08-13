//! Focused proof for the shared source preflight.

#![allow(dead_code, unused_imports)]

use super::{PreflightReport, SourceState, preflight};
use crate::source::{CatalogState, RevisionId, SourceCatalog, SourceId};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn catalog() -> SourceCatalog {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: source("primary"),
            revision: revision("rev-1"),
        })
        .expect("record");
    catalog
        .record(CatalogState::Unavailable {
            source: source("broken"),
            reason: "acquisition failed".to_owned(),
        })
        .expect("record");
    catalog
}

#[test]
fn invalid_machine_state_admits_no_effect() {
    let report = preflight(
        &catalog(),
        false,
        &[
            ("dest-a", &source("primary")),
            ("dest-b", &source("broken")),
        ],
    );
    assert!(!report.machine_valid);
    assert!(
        report.eligible.is_empty(),
        "no source or destination effect"
    );
    assert_eq!(report.affected.len(), 2);
}

#[test]
fn source_diagnostics_name_precedence_and_affected_plans() {
    let report = preflight(
        &catalog(),
        true,
        &[
            ("dest-a", &source("primary")),
            ("dest-b", &source("broken")),
        ],
    );
    assert_eq!(report.sources.len(), 2);
    let primary = &report.sources[0];
    assert_eq!(primary.position, 1, "declared precedence position");
    assert_eq!(primary.state, SourceState::Complete);
    assert_eq!(primary.affected_plans, vec!["dest-a".to_owned()]);
    let broken = &report.sources[1];
    assert_eq!(broken.position, 2);
    assert!(
        matches!(&broken.state, SourceState::Unavailable { reason } if reason.contains("acquisition failed"))
    );
    assert_eq!(broken.affected_plans, vec!["dest-b".to_owned()]);
}

#[test]
fn independent_repositories_remain_eligible_under_policy() {
    let report = preflight(
        &catalog(),
        true,
        &[
            ("dest-a", &source("primary")),
            ("dest-b", &source("broken")),
        ],
    );
    assert_eq!(report.eligible, vec!["dest-a".to_owned()]);
    assert_eq!(report.affected, vec!["dest-b".to_owned()]);
}

#[test]
fn shadowed_sources_diagnose_the_hiding_source() {
    let mut catalog = catalog();
    catalog
        .record(CatalogState::Shadowed {
            source: source("legacy"),
            by: source("primary"),
        })
        .expect("record");
    let report = preflight(&catalog, true, &[("dest-c", &source("legacy"))]);
    let legacy = &report.sources[2];
    assert_eq!(legacy.position, 3);
    assert_eq!(
        legacy.state,
        SourceState::Shadowed {
            by: source("primary")
        }
    );
    assert_eq!(report.eligible.len(), 0);
    assert_eq!(report.affected, vec!["dest-c".to_owned()]);
}
