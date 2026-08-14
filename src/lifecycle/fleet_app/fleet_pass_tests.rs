//! Focused proof for composing leases, scheduler, and the initial fleet
//! pass without repository-specific coupling.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_app::run_fleet_pass;
use crate::lifecycle::fleet_collector::MemberResult;
use crate::lifecycle::fleet_fanout::RepoResult;
use crate::lifecycle::work_mapping::WorkItem;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

fn work(id: &str) -> WorkItem {
    WorkItem::Run {
        repository: id.to_owned(),
        plan_identity: "plan-1".to_owned(),
    }
}

#[test]
fn lease_denied_repositories_are_accounted_not_dropped() {
    // One repository already holds a lease elsewhere, so its acquisition
    // is denied; the pass still accounts it.
    let items: Vec<WorkItem> = vec![work("free-a"), work("busy-b")];
    let response = run_fleet_pass(
        "run-1",
        &items,
        2,
        &mut |repository: &str| !repository.starts_with("busy"),
        |item| match item {
            WorkItem::Run { repository, .. } => RepoResult::Delivered {
                repository: repository.clone(),
                oid: "oid".to_owned(),
            },
            WorkItem::Skip { repository, .. } => RepoResult::Skipped {
                repository: repository.clone(),
                reason: "preflight".to_owned(),
            },
        },
    )
    .expect("pass");
    assert_eq!(response.results.len(), 2, "every repository is accounted");
    assert!(matches!(
        response.results[0],
        MemberResult::Delivered { .. }
    ));
    assert!(matches!(response.results[1], MemberResult::Failed { .. }));
}

#[test]
fn bounded_scheduling_is_respected() {
    let items: Vec<WorkItem> = (0..6).map(|i| work(&format!("dest-{i}"))).collect();
    let observed = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let observed_for_runner = Arc::clone(&observed);
    let peak_for_runner = Arc::clone(&peak);
    let response = run_fleet_pass(
        "run-1",
        &items,
        2,
        &mut |_repository: &str| true,
        move |item| {
            let repository = match item {
                WorkItem::Run { repository, .. } => repository.clone(),
                WorkItem::Skip { repository, .. } => repository.clone(),
            };
            let current = observed_for_runner.fetch_add(1, Ordering::SeqCst) + 1;
            peak_for_runner.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            observed_for_runner.fetch_sub(1, Ordering::SeqCst);
            RepoResult::Delivered {
                repository,
                oid: "oid".to_owned(),
            }
        },
    )
    .expect("pass");
    assert!(peak.load(Ordering::SeqCst) <= 2, "bounded concurrency");
    assert_eq!(response.results.len(), 6);
}

#[test]
fn the_pass_has_no_repository_specific_coupling() {
    // The same injected runner and lease check drive any work item shape:
    // nothing in the composition names a repository.
    let items: Vec<WorkItem> = vec![
        work("alpha"),
        WorkItem::Skip {
            repository: "beta".to_owned(),
            reason: "preflight".to_owned(),
        },
    ];
    let response =
        run_fleet_pass(
            "run-1",
            &items,
            2,
            &mut |_repository: &str| true,
            |item| match item {
                WorkItem::Run { repository, .. } => RepoResult::Delivered {
                    repository: repository.clone(),
                    oid: "oid".to_owned(),
                },
                WorkItem::Skip { repository, .. } => RepoResult::Skipped {
                    repository: repository.clone(),
                    reason: "preflight".to_owned(),
                },
            },
        )
        .expect("pass");
    assert_eq!(response.results.len(), 2);
    assert!(matches!(
        response.results[0],
        MemberResult::Delivered { .. }
    ));
    assert!(matches!(response.results[1], MemberResult::Skipped { .. }));
}
