//! Focused proof for catalog states and planning impact.

#![allow(dead_code, unused_imports)]

use super::{CatalogError, CatalogState, PlanningImpact, SourceCatalog, plan_impact};
use crate::source::snapshot::{RevisionId, SourceId};

fn source(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}
fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

#[test]
fn catalog_distinguishes_complete_shadowed_and_unavailable() {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: source("primary"),
            revision: revision("rev-1"),
        })
        .expect("record");
    catalog
        .record(CatalogState::Shadowed {
            source: source("legacy"),
            by: source("primary"),
        })
        .expect("record");
    catalog
        .record(CatalogState::Unavailable {
            source: source("broken"),
            reason: "acquisition failed".to_owned(),
        })
        .expect("record");
    let entries = catalog.entries();
    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0], CatalogState::Complete { .. }));
    assert!(matches!(entries[1], CatalogState::Shadowed { .. }));
    assert!(matches!(entries[2], CatalogState::Unavailable { .. }));
}

#[test]
fn duplicate_sources_fail_typed() {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: source("primary"),
            revision: revision("rev-1"),
        })
        .expect("record");
    let error = catalog
        .record(CatalogState::Unavailable {
            source: source("primary"),
            reason: "late failure".to_owned(),
        })
        .expect_err("duplicate");
    assert!(matches!(error, CatalogError::Duplicate { .. }), "{error}");
}

#[test]
fn unavailable_sources_affect_only_dependent_repositories() {
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
    let impacts = plan_impact(
        &catalog,
        &[
            ("dest-a", &source("primary")),
            ("dest-b", &source("broken")),
        ],
    );
    assert_eq!(
        impacts[0],
        PlanningImpact::Independent {
            repository: "dest-a".to_owned()
        }
    );
    match &impacts[1] {
        PlanningImpact::Affected { repository, reason } => {
            assert_eq!(repository, "dest-b");
            assert!(reason.contains("unavailable"), "{reason}");
        }
        other => panic!("expected affected, got {other:?}"),
    }
}

#[test]
fn shadowed_sources_never_promote_and_stay_explainable() {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: source("primary"),
            revision: revision("rev-1"),
        })
        .expect("record");
    catalog
        .record(CatalogState::Shadowed {
            source: source("legacy"),
            by: source("primary"),
        })
        .expect("record");
    let impacts = plan_impact(&catalog, &[("dest-a", &source("legacy"))]);
    match &impacts[0] {
        PlanningImpact::Affected { repository, reason } => {
            assert_eq!(repository, "dest-a");
            assert!(reason.contains("shadowed"), "{reason}");
        }
        other => panic!("expected affected, got {other:?}"),
    }
}

#[test]
fn undeclared_sources_are_affected_not_ignored() {
    let catalog = SourceCatalog::new();
    let impacts = plan_impact(&catalog, &[("dest-a", &source("ghost"))]);
    match &impacts[0] {
        PlanningImpact::Affected { repository, reason } => {
            assert_eq!(repository, "dest-a");
            assert!(reason.contains("not declared"), "{reason}");
        }
        other => panic!("expected affected, got {other:?}"),
    }
}
