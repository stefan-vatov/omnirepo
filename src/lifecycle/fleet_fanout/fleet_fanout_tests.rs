//! Focused proof for bounded fleet fan-out of independent passes.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_fanout::{FanoutReport, RepoResult, fan_out};
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
fn bounded_scheduling_never_exceeds_the_limit_and_every_item_reaches_a_result() {
    let items: Vec<WorkItem> = (0..6).map(|i| work(&format!("dest-{i}"))).collect();
    let observed = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let observed_for_runner = Arc::clone(&observed);
    let peak_for_runner = Arc::clone(&peak);
    let report = fan_out(&items, 2, move |_item| {
        let current = observed_for_runner.fetch_add(1, Ordering::SeqCst) + 1;
        peak_for_runner.fetch_max(current, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(2));
        observed_for_runner.fetch_sub(1, Ordering::SeqCst);
        RepoResult::Delivered {
            repository: "dest".to_owned(),
            oid: "oid".to_owned(),
        }
    });
    assert!(peak.load(Ordering::SeqCst) <= 2, "bounded concurrency");
    assert_eq!(report.results.len(), 6, "every item reaches a result");
}

#[test]
fn a_failed_pass_does_not_stop_independent_peers() {
    let items = vec![work("dest-a"), work("dest-b"), work("dest-c")];
    let report = fan_out(&items, 3, |item| match item {
        WorkItem::Run { repository, .. } if repository == "dest-b" => RepoResult::Failed {
            repository: repository.clone(),
            reason: "verify failed".to_owned(),
        },
        WorkItem::Run { repository, .. } => RepoResult::Delivered {
            repository: repository.clone(),
            oid: "oid".to_owned(),
        },
        WorkItem::Skip { repository, .. } => RepoResult::Skipped {
            repository: repository.clone(),
            reason: "skipped".to_owned(),
        },
    });
    assert_eq!(report.results.len(), 3);
    let failed = report
        .results
        .iter()
        .filter(|result| matches!(result, RepoResult::Failed { .. }))
        .count();
    assert_eq!(failed, 1, "one failure");
    assert_eq!(
        report
            .results
            .iter()
            .filter(|result| matches!(result, RepoResult::Delivered { .. }))
            .count(),
        2,
        "both peers delivered"
    );
}

#[test]
fn skipped_work_items_pass_through() {
    let items = vec![
        WorkItem::Skip {
            repository: "dest-z".to_owned(),
            reason: "preflight failed".to_owned(),
        },
        work("dest-a"),
    ];
    let report = fan_out(&items, 2, |item| match item {
        WorkItem::Run { repository, .. } => RepoResult::Delivered {
            repository: repository.clone(),
            oid: "oid".to_owned(),
        },
        WorkItem::Skip { repository, .. } => RepoResult::Skipped {
            repository: repository.clone(),
            reason: "skipped".to_owned(),
        },
    });
    assert_eq!(report.results.len(), 2);
    assert!(matches!(report.results[0], RepoResult::Skipped { .. }));
    assert!(matches!(report.results[1], RepoResult::Delivered { .. }));
}

#[test]
fn empty_work_yields_an_empty_report() {
    let report = fan_out(&[], 2, |_item| RepoResult::Delivered {
        repository: String::new(),
        oid: String::new(),
    });
    let _: FanoutReport = report;
    assert!(report.results.is_empty());
}
