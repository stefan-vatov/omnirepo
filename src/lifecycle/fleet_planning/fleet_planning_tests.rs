//! Focused proof for building per-repository plans with affected naming.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, DestinationRepository, MachineConcurrency, MachineConfiguration, RepairControls,
    RepositoryId, SchemaVersion,
};
use crate::lifecycle::fleet_planning::build_repository_plans;
use crate::lifecycle::fleet_policy::RepositoryPolicyLoad;
use crate::lifecycle::plan_selection::Policy;
use crate::source::{CatalogState, ItemDeclaration, ItemKind, RevisionId, SourceCatalog, SourceId};
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-planning-")
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

fn item(id: &str, source: &str, order: usize) -> ItemDeclaration {
    ItemDeclaration {
        id: id.to_owned(),
        target: format!("apps/{id}.yaml"),
        source: source.to_owned(),
        kind: ItemKind::WholeFile,
        section: None,
        source_order: order,
    }
}

fn absent_policy() -> RepositoryPolicyLoad {
    RepositoryPolicyLoad {
        repository: String::new(),
        policy: Some(Policy::Absent),
        failure: None,
    }
}

#[test]
fn plans_build_with_items_in_declared_order_under_absent_policy() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let catalog = complete_catalog("source-a");
    let declarations = vec![(
        "repo-a".to_owned(),
        vec![item("item-1", "source-a", 0), item("item-2", "source-a", 1)],
    )];
    let policies = vec![absent_policy()];
    let plans = build_repository_plans(&config, &catalog, &declarations, &policies);
    assert_eq!(plans.len(), 1);
    let plan = plans[0].plan.as_ref().expect("plan");
    assert_eq!(plan.destination, "repo-a");
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.items[0].id, "item-1");
    assert_eq!(plan.items[1].id, "item-2", "declared order preserved");
    assert!(
        plan.items.iter().all(|item| {
            matches!(
                item.decision,
                crate::lifecycle::sync_plan::PlanDecision::Selected { .. }
            )
        }),
        "absent policy infers every declared item"
    );
}

#[test]
fn an_unavailable_source_names_the_affected_source_and_item() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    // The catalog records the source as unavailable.
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Unavailable {
            source: SourceId::new("source-a").expect("source"),
            reason: "remote materialization is not available for this run".to_owned(),
        })
        .expect("record");
    let declarations = vec![("repo-a".to_owned(), vec![item("item-1", "source-a", 0)])];
    let policies = vec![absent_policy()];
    let plans = build_repository_plans(&config, &catalog, &declarations, &policies);
    let error = plans[0].plan.as_ref().expect_err("affected");
    assert!(error.contains("source-a"), "{error}");
    assert!(error.contains("item-1"), "{error}");
}

#[test]
fn a_policy_failure_fails_only_its_plan_and_peers_continue() {
    let fixture = fixture_base();
    let config = machine(vec![
        destination(fixture.path(), "repo-a"),
        destination(fixture.path(), "repo-b"),
    ]);
    let catalog = complete_catalog("source-a");
    let declarations = vec![
        ("repo-a".to_owned(), vec![item("item-1", "source-a", 0)]),
        ("repo-b".to_owned(), vec![item("item-2", "source-a", 0)]),
    ];
    let policies = vec![
        RepositoryPolicyLoad {
            repository: "repo-a".to_owned(),
            policy: None,
            failure: Some("aliased policy file".to_owned()),
        },
        absent_policy(),
    ];
    let plans = build_repository_plans(&config, &catalog, &declarations, &policies);
    assert_eq!(plans.len(), 2);
    assert!(plans[0].plan.is_err(), "repo-a fails typed");
    assert!(plans[1].plan.is_ok(), "repo-b continues");
    let plan = plans[1].plan.as_ref().expect("plan");
    assert_eq!(plan.items[0].id, "item-2");
}

#[test]
fn explicit_policy_selection_governs_the_plan() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let catalog = complete_catalog("source-a");
    let declarations = vec![(
        "repo-a".to_owned(),
        vec![item("item-1", "source-a", 0), item("item-2", "source-a", 1)],
    )];
    let policies = vec![RepositoryPolicyLoad {
        repository: "repo-a".to_owned(),
        policy: Some(Policy::Explicit {
            include: vec!["item-1".to_owned()],
            exclude: Vec::new(),
        }),
        failure: None,
    }];
    let plans = build_repository_plans(&config, &catalog, &declarations, &policies);
    let plan = plans[0].plan.as_ref().expect("plan");
    // Only the included item is selected; the other is rejected by the
    // explicit policy (never inferred).
    let selected = plan
        .items
        .iter()
        .filter(|item| {
            matches!(
                item.decision,
                crate::lifecycle::sync_plan::PlanDecision::Selected { .. }
            )
        })
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(selected, vec!["item-1"], "{selected:?}");
}
