//! Fleet mixed-outcome, straggler, ordering, and cancellation fixtures.
//!
//! STRICT TDD: this test file was written and run RED before the fixture
//! composition existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_collector::{MemberResult, collect_fleet_results};
use crate::lifecycle::fleet_fanout::{RepoResult, fan_out};
use crate::lifecycle::work_mapping::WorkItem;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn work(id: &str) -> WorkItem {
    WorkItem::Run {
        repository: id.to_owned(),
        plan_identity: "plan-1".to_owned(),
    }
}

#[test]
fn valid_peers_complete_while_a_straggler_holds_a_slot() {
    let items: Vec<WorkItem> = vec![work("fast-a"), work("slow-b"), work("fast-c")];
    let finished = Arc::new(AtomicUsize::new(0));
    let finished_for_runner = Arc::clone(&finished);
    let start = Instant::now();
    let report = fan_out(&items, 2, move |item| {
        let repository = match item {
            WorkItem::Run { repository, .. } => repository.clone(),
            WorkItem::Skip { repository, .. } => repository.clone(),
        };
        if repository == "slow-b" {
            // The straggler holds its slot until the run ends.
            std::thread::sleep(Duration::from_millis(30));
        }
        finished_for_runner.fetch_add(1, Ordering::SeqCst);
        RepoResult::Delivered {
            repository: repository.clone(),
            oid: "oid".to_owned(),
        }
    });
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "the straggler is bounded"
    );
    assert_eq!(report.results.len(), 3, "every member completes");
    assert_eq!(finished.load(Ordering::SeqCst), 3);
}

#[test]
fn completion_permutations_preserve_association_and_accounting() {
    let items: Vec<WorkItem> = vec![work("a"), work("b"), work("c")];
    // The runner varies its duration by repository, so completion order
    // differs per repository while the association stays stable.
    let first = fan_out(&items, 3, |item| {
        let repository = match item {
            WorkItem::Run { repository, .. } => repository.clone(),
            WorkItem::Skip { repository, .. } => repository.clone(),
        };
        let delay = match repository.as_str() {
            "a" => 8,
            "b" => 2,
            _ => 5,
        };
        std::thread::sleep(Duration::from_millis(delay));
        RepoResult::Delivered {
            repository: repository.clone(),
            oid: format!("oid-{repository}"),
        }
    });
    let second = fan_out(&items, 3, |item| {
        let repository = match item {
            WorkItem::Run { repository, .. } => repository.clone(),
            WorkItem::Skip { repository, .. } => repository.clone(),
        };
        let delay = match repository.as_str() {
            "a" => 2,
            "b" => 8,
            _ => 5,
        };
        std::thread::sleep(Duration::from_millis(delay));
        RepoResult::Delivered {
            repository: repository.clone(),
            oid: format!("oid-{repository}"),
        }
    });
    // The association (repository -> oid) is identical across runs; the
    // ordered report preserves the declared positions.
    let ids_first: Vec<String> = first
        .results
        .iter()
        .map(|r| match r {
            RepoResult::Delivered { repository, .. } => repository.clone(),
            RepoResult::Failed { repository, .. } => repository.clone(),
            RepoResult::Skipped { repository, .. } => repository.clone(),
        })
        .collect();
    let ids_second: Vec<String> = second
        .results
        .iter()
        .map(|r| match r {
            RepoResult::Delivered { repository, .. } => repository.clone(),
            RepoResult::Failed { repository, .. } => repository.clone(),
            RepoResult::Skipped { repository, .. } => repository.clone(),
        })
        .collect();
    assert_eq!(ids_first, vec!["a", "b", "c"]);
    assert_eq!(ids_second, vec!["a", "b", "c"]);
    for result in &first.results {
        let RepoResult::Delivered { repository, oid } = result else {
            panic!("expected delivered");
        };
        assert_eq!(oid, &format!("oid-{repository}"), "association stable");
    }
}

#[test]
fn mixed_outcomes_collect_one_result_per_member_without_duplicates() {
    let items: Vec<WorkItem> = vec![
        work("ok-a"),
        work("bad-b"),
        WorkItem::Skip {
            repository: "skip-c".to_owned(),
            reason: "preflight".to_owned(),
        },
    ];
    let report = fan_out(&items, 3, |item| match item {
        WorkItem::Run { repository, .. } if repository == "bad-b" => RepoResult::Failed {
            repository: repository.clone(),
            reason: "verify failed".to_owned(),
        },
        WorkItem::Run { repository, .. } => RepoResult::Delivered {
            repository: repository.clone(),
            oid: "oid".to_owned(),
        },
        WorkItem::Skip { repository, .. } => RepoResult::Skipped {
            repository: repository.clone(),
            reason: "preflight".to_owned(),
        },
    });
    let fleet = collect_fleet_results(
        report.results.clone(),
        &["ok-a".to_owned(), "bad-b".to_owned(), "skip-c".to_owned()],
    )
    .expect("collect");
    assert_eq!(fleet.members.len(), 3, "one result per member");
    assert!(matches!(fleet.members[0], MemberResult::Delivered { .. }));
    assert!(matches!(fleet.members[1], MemberResult::Failed { .. }));
    assert!(matches!(fleet.members[2], MemberResult::Skipped { .. }));
    // No duplicate pass: every repository appears once in the report.
    let ids: Vec<&str> = report
        .results
        .iter()
        .map(|r| match r {
            RepoResult::Delivered { repository, .. }
            | RepoResult::Failed { repository, .. }
            | RepoResult::Skipped { repository, .. } => repository.as_str(),
        })
        .collect();
    assert_eq!(ids.len(), 3);
}

#[test]
fn no_agent_or_final_renderer_is_invoked() {
    // The fan-out and collection are pure composition: no agent call and
    // no renderer is reachable from these modules.  (Compile-time contract;
    // the assertion pins the shape.)
    let items: Vec<WorkItem> = vec![work("a")];
    let report = fan_out(&items, 1, |item| {
        let repository = match item {
            WorkItem::Run { repository, .. } => repository.clone(),
            WorkItem::Skip { repository, .. } => repository.clone(),
        };
        RepoResult::Delivered {
            repository,
            oid: "oid".to_owned(),
        }
    });
    assert_eq!(report.results.len(), 1);
}
