//! Saturation, cancellation, backpressure, and leak fixtures.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_permits::FleetPermits;
use crate::lifecycle::nested_permits::{NestedPermits, WorkKind};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

/// Observed concurrency never exceeds the configured limit under
/// saturation: many workers compete for few slots.
#[test]
fn saturation_never_exceeds_the_configured_limit() {
    let permits = NestedPermits::new(3, &[(WorkKind::Git, 3)]).expect("ledger");
    let observed = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for worker in 0..10 {
        let permits = permits.clone();
        let observed = Arc::clone(&observed);
        let peak = Arc::clone(&peak);
        handles.push(std::thread::spawn(move || {
            let permit = loop {
                if let Some(permit) = permits
                    .acquire(WorkKind::Git, format!("repo-{worker}"))
                    .expect("grant")
                {
                    break permit;
                }
                std::thread::sleep(Duration::from_millis(1));
            };
            let current = observed.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            observed.fetch_sub(1, Ordering::SeqCst);
            permits.release(&permit, true).expect("release");
        }));
    }
    for handle in handles {
        let _ = handle.join();
    }
    assert!(
        peak.load(Ordering::SeqCst) <= 3,
        "concurrency must never exceed the limit"
    );
    assert_eq!(permits.active_repositories(), 0, "all permits returned");
}

/// All permits return after many grant/release cycles: no leak.
#[test]
fn all_permits_return_after_many_cycles() {
    let permits = FleetPermits::new(4).expect("ledger");
    for cycle in 0..200 {
        permits.enqueue(format!("repo-{cycle}"));
        if let Some(permit) = permits.grant_next().expect("grant") {
            drop(permit);
        }
    }
    assert_eq!(permits.active(), 0, "no permit leaks");
    assert_eq!(permits.queued(), 0);
}

/// Cancellation under load: workers that already hold permits finish and
/// release; new acquisition stops typed; every repository is accounted.
#[test]
fn cancellation_under_load_accounts_every_repository() {
    let permits = NestedPermits::new(2, &[(WorkKind::Agent, 2)]).expect("ledger");
    let finished = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for name in ["a", "b"] {
        let permits = permits.clone();
        let finished = Arc::clone(&finished);
        handles.push(std::thread::spawn(move || {
            let permit = permits
                .acquire(WorkKind::Agent, name)
                .expect("grant")
                .expect("permit");
            std::thread::sleep(Duration::from_millis(5));
            permits.release(&permit, true).expect("release");
            finished.fetch_add(1, Ordering::SeqCst);
        }));
    }
    // Wait until both slots are taken, then cancel.
    while permits.active_repositories() < 2 {
        std::thread::sleep(Duration::from_millis(1));
    }
    permits.cancel();
    // New acquisition is refused typed after cancellation.
    assert!(permits.acquire(WorkKind::Agent, "c").is_err());
    for handle in handles {
        let _ = handle.join();
    }
    assert_eq!(
        finished.load(Ordering::SeqCst),
        2,
        "in-flight work completes"
    );
    assert_eq!(permits.active_repositories(), 0);
}
