//! Focused proof for mapping preflight results into scheduler work items.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repository_preflight::RepoPreflight;
use crate::lifecycle::work_mapping::{WorkItem, map_preflight_to_work};

#[test]
fn every_preflight_repository_maps_to_exactly_one_work_item() {
    let preflight = vec![
        RepoPreflight::ReadyPlan {
            repository: "dest-a".to_owned(),
        },
        RepoPreflight::Failed {
            repository: "dest-b".to_owned(),
            reasons: vec!["source unavailable".to_owned()],
        },
        RepoPreflight::ReadyPlan {
            repository: "dest-c".to_owned(),
        },
    ];
    let work = map_preflight_to_work(&preflight);
    assert_eq!(work.len(), 3, "every repository maps exactly once");
    assert!(matches!(work[0], WorkItem::Run { .. }));
    assert!(matches!(work[1], WorkItem::Skip { .. }));
    assert!(matches!(work[2], WorkItem::Run { .. }));
}

#[test]
fn ready_plans_become_run_items_with_the_plan_identity() {
    let preflight = vec![RepoPreflight::ReadyPlan {
        repository: "dest-a".to_owned(),
    }];
    let work = map_preflight_to_work(&preflight);
    match &work[0] {
        WorkItem::Run {
            repository,
            plan_identity,
        } => {
            assert_eq!(repository, "dest-a");
            assert!(!plan_identity.is_empty());
        }
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn failed_repositories_become_skips_with_the_reason() {
    let preflight = vec![RepoPreflight::Failed {
        repository: "dest-b".to_owned(),
        reasons: vec!["source unavailable".to_owned(), "shadowed".to_owned()],
    }];
    let work = map_preflight_to_work(&preflight);
    match &work[0] {
        WorkItem::Skip { repository, reason } => {
            assert_eq!(repository, "dest-b");
            assert!(reason.contains("source unavailable"), "{reason}");
            assert!(reason.contains("shadowed"), "{reason}");
        }
        other => panic!("expected skip, got {other:?}"),
    }
}

#[test]
fn declared_order_is_preserved() {
    let preflight = vec![
        RepoPreflight::Failed {
            repository: "dest-z".to_owned(),
            reasons: vec!["x".to_owned()],
        },
        RepoPreflight::ReadyPlan {
            repository: "dest-a".to_owned(),
        },
    ];
    let work = map_preflight_to_work(&preflight);
    let labels: Vec<String> = work
        .iter()
        .map(|item| match item {
            WorkItem::Run { repository, .. } => repository.clone(),
            WorkItem::Skip { repository, .. } => repository.clone(),
        })
        .collect();
    assert_eq!(
        labels,
        vec!["dest-z".to_owned(), "dest-a".to_owned()],
        "declared order kept"
    );
}
