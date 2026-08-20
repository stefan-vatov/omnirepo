//! Focused proof for freezing machine concurrency and composing the
//! fleet.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, DestinationRepository, MachineConcurrency, MachineConfiguration, RepairControls,
    RepositoryId, SchemaVersion,
};
use crate::lifecycle::fleet_composition::{
    CompositionOutcome, compose_configured_fleet, freeze_concurrency,
};
use crate::lifecycle::fleet_planning::RepositoryPlan;
use crate::lifecycle::sync_plan::PlanDecision;
use crate::lifecycle::sync_plan::{PlanItem, SyncPlan};
use crate::source::{CatalogState, ItemKind, RevisionId, SourceCatalog, SourceId};
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-composition-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn destination(root: &Path, id: &str) -> DestinationRepository {
    let path = root.join(id);
    fs::create_dir_all(&path).expect("destination");
    DestinationRepository::new(
        RepositoryId::parse(id).expect("repository id"),
        AbsolutePath::parse(path.to_str().expect("utf8")).expect("path"),
        Vec::new(),
    )
    .expect("destination")
}

fn machine(repositories: Vec<DestinationRepository>) -> MachineConfiguration {
    MachineConfiguration::new(
        SchemaVersion::new(1).expect("version"),
        repositories,
        Vec::new(),
        None,
        MachineConcurrency::new(4, 8).expect("concurrency"),
        RepairControls::default(),
    )
    .expect("machine")
}

fn complete_catalog(source: &str) -> SourceCatalog {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: SourceId::new(source).expect("source"),
            revision: RevisionId::new("rev-1").expect("revision"),
        })
        .expect("record");
    catalog
}

fn item(id: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: format!("apps/{id}.yaml"),
        source: "source-a".to_owned(),
        source_path: String::new(),
        source_order: 0,
        kind: ItemKind::WholeFile,
        section: None,
        decision: PlanDecision::Selected {
            reason: "inferred".to_owned(),
        },
    }
}

fn ok_plan(repository: &str, items: Vec<PlanItem>) -> RepositoryPlan {
    RepositoryPlan {
        repository: repository.to_owned(),
        plan: Ok(SyncPlan::new(repository, items)),
        checks: Vec::new(),
    }
}

fn failed_plan(repository: &str, reason: &str) -> RepositoryPlan {
    RepositoryPlan {
        repository: repository.to_owned(),
        plan: Err(reason.to_owned()),
        checks: Vec::new(),
    }
}

#[test]
fn concurrency_defaults_freeze_and_overrides_only_lower() {
    let machine = MachineConcurrency::new(8, 16).expect("machine");
    // No override: the machine cap freezes.
    assert_eq!(freeze_concurrency(machine, None).expect("freeze"), 8);
    // A lower override is accepted for the run.
    assert_eq!(freeze_concurrency(machine, Some(2)).expect("freeze"), 2);
    // An equal override is accepted.
    assert_eq!(freeze_concurrency(machine, Some(8)).expect("freeze"), 8);
    // A higher override is an invocation error and never raises the cap.
    assert!(freeze_concurrency(machine, Some(16)).is_err());
    // The default machine (4) freezes when the config omits the mapping.
    let default = MachineConcurrency::new(4, 8).expect("default");
    assert_eq!(freeze_concurrency(default, None).expect("freeze"), 4);
}

#[test]
fn the_fleet_composes_from_the_plans_in_declared_order() {
    let fixture = fixture_base();
    let config = machine(vec![
        destination(fixture.path(), "repo-a"),
        destination(fixture.path(), "repo-b"),
    ]);
    let catalog = complete_catalog("source-a");
    let plans = vec![
        ok_plan("repo-a", vec![item("item-1")]),
        ok_plan("repo-b", vec![item("item-2")]),
    ];
    let outcome = compose_configured_fleet(&config, &catalog, &plans, None).expect("compose");
    assert_eq!(outcome.composition.accounted.len(), 2);
    assert_eq!(outcome.composition.work.len(), 2, "both repos have work");
    let ids = outcome
        .composition
        .work
        .iter()
        .map(|item| match item {
            crate::lifecycle::work_mapping::WorkItem::Run { repository, .. } => repository.as_str(),
            crate::lifecycle::work_mapping::WorkItem::Skip { repository, .. } => {
                repository.as_str()
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-a", "repo-b"], "declared order preserved");
    assert_eq!(outcome.limit, 4, "the default machine limit freezes");
}

#[test]
fn a_failed_plan_affects_only_its_repository() {
    let fixture = fixture_base();
    let config = machine(vec![
        destination(fixture.path(), "repo-a"),
        destination(fixture.path(), "repo-b"),
    ]);
    let catalog = complete_catalog("source-a");
    let plans = vec![
        failed_plan(
            "repo-a",
            "source-a item-1 is affected by an unavailable source",
        ),
        ok_plan("repo-b", vec![item("item-2")]),
    ];
    let outcome = compose_configured_fleet(&config, &catalog, &plans, None).expect("compose");
    assert_eq!(outcome.composition.work.len(), 1, "repo-b works");
    assert!(
        outcome
            .composition
            .affected
            .iter()
            .any(|entry| entry.starts_with("repo-a")),
        "repo-a is affected: {:?}",
        outcome.composition.affected
    );
    // The accounting covers the whole fleet: the workable repo is
    // accounted and the affected repo is named in the affected list.
    assert_eq!(outcome.composition.accounted.len(), 1);
    assert_eq!(outcome.composition.affected.len(), 1);
}

#[test]
fn an_empty_fleet_composes_as_success() {
    let config = machine(Vec::new());
    let catalog = complete_catalog("source-a");
    let outcome = compose_configured_fleet(&config, &catalog, &[], None).expect("compose");
    assert!(outcome.composition.work.is_empty());
    assert!(outcome.composition.accounted.is_empty());
    assert_eq!(outcome.limit, 4);
}
