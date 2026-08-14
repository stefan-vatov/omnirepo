//! Bounded fleet fan-out of independent one-repository passes.
//!
//! The ready work items run under a bounded concurrency limit; each
//! repository is independent, so a failure never stops its peers; every
//! work item reaches exactly one result in the final report.

#![allow(dead_code)]

use crate::lifecycle::work_mapping::WorkItem;

#[cfg(test)]
mod fleet_fanout_tests;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// One repository's final result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepoResult {
    Delivered { repository: String, oid: String },
    Failed { repository: String, reason: String },
    Skipped { repository: String, reason: String },
}

/// The fan-out report (declared order preserved).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FanoutReport {
    pub results: Vec<RepoResult>,
}

/// Run the ready work items under the bounded limit.  `runner` executes
/// one item and returns its result; peers are never affected by a peer's
/// failure.
pub fn fan_out(
    items: &[WorkItem],
    limit: usize,
    runner: impl Fn(&WorkItem) -> RepoResult + Send + Sync + 'static,
) -> FanoutReport {
    if limit == 0 || items.is_empty() {
        return FanoutReport {
            results: Vec::new(),
        };
    }
    let index = Arc::new(AtomicUsize::new(0));
    let results: Arc<Mutex<Vec<(usize, RepoResult)>>> = Arc::new(Mutex::new(Vec::new()));
    // The items and the runner are shared with the worker threads through
    // owned Arc clones (cheap: the items carry small strings).
    let shared: Arc<Vec<WorkItem>> = Arc::new(items.to_vec());
    let runner = Arc::new(runner);
    let workers: Vec<_> = (0..limit)
        .map(|_| {
            let index = Arc::clone(&index);
            let results = Arc::clone(&results);
            let shared = Arc::clone(&shared);
            let runner = Arc::clone(&runner);
            std::thread::spawn(move || {
                loop {
                    let position = index.fetch_add(1, Ordering::SeqCst);
                    if position >= shared.len() {
                        break;
                    }
                    let result = runner(&shared[position]);
                    results.lock().expect("results").push((position, result));
                }
            })
        })
        .collect();
    for worker in workers {
        let _ = worker.join();
    }
    let mut ordered = results.lock().expect("results").clone();
    ordered.sort_by_key(|(position, _)| *position);
    FanoutReport {
        results: ordered.into_iter().map(|(_, result)| result).collect(),
    }
}
