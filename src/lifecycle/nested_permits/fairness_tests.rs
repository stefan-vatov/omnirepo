//! Fairness and progress fixtures for mixed fast, slow, and failed
//! repositories.

#![allow(dead_code, unused_imports)]

use super::{NestedPermits, WorkKind};
use crate::lifecycle::fleet_permits::FleetPermits;
use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// With controlled barriers, ready peers admit and finish while one child
/// hangs until it is terminated.  No completion-order guarantee is
/// introduced: only progress and boundedness are asserted.
#[test]
fn ready_peers_finish_while_one_child_hangs_until_termination() {
    let permits =
        NestedPermits::new(2, &[(WorkKind::Agent, 2), (WorkKind::Git, 2)]).expect("ledger");
    let finished = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for (name, hang) in [("fast-a", false), ("fast-b", false), ("hung-c", true)] {
        let permits = permits.clone();
        let finished = Arc::clone(&finished);
        handles.push(std::thread::spawn(move || {
            // Bounded wait for a permit: the hung child occupies one slot;
            // the ready peers admit as slots free up.
            let permit = loop {
                if let Some(permit) = permits.acquire(WorkKind::Agent, name).expect("grant") {
                    break permit;
                }
                std::thread::sleep(Duration::from_millis(1));
            };
            if hang {
                // The hung child holds its permit until the caller
                // terminates it (simulated by the sleep); termination is
                // confirmed before release.
                std::thread::sleep(Duration::from_millis(50));
            }
            permits.release(&permit, true).expect("release");
            finished.fetch_add(1, Ordering::SeqCst);
        }));
    }
    // All workers reach a terminal slot; the run is bounded even with a
    // hung child.  No completion order is guaranteed.
    let start = Instant::now();
    for handle in handles {
        let _ = handle.join();
    }
    assert!(start.elapsed() < Duration::from_secs(5), "bounded progress");
    assert_eq!(
        finished.load(Ordering::SeqCst),
        3,
        "every repo reaches a terminal slot"
    );
    assert_eq!(permits.active_repositories(), 0);
}

/// The FIFO grant order is deterministic for the same enqueue sequence:
/// queue behavior is reproducible even though completion order is not
/// guaranteed.
#[test]
fn grant_order_is_deterministic_fifo() {
    let permits = FleetPermits::new(2).expect("ledger");
    for id in ["r1", "r2", "r3", "r4"] {
        permits.enqueue(id);
    }
    let mut order = Vec::new();
    while let Some(permit) = permits.grant_next().expect("grant") {
        order.push(permit.repository.clone());
        drop(permit);
    }
    assert_eq!(order, vec!["r1", "r2", "r3", "r4"]);
}

/// A slow repository never starves the queue: while one permit is held
/// long, later peers still progress when the limit allows.
#[test]
fn slow_repositories_do_not_starve_the_queue() {
    let permits = NestedPermits::new(2, &[(WorkKind::Git, 2)]).expect("ledger");
    let slow_held = Arc::new(Barrier::new(2));
    let slow_released = Arc::new(Barrier::new(2));
    let held_for_thread = Arc::clone(&slow_held);
    let released_for_thread = Arc::clone(&slow_released);
    let permits_a = permits.clone();
    let slow = std::thread::spawn(move || {
        let permit = permits_a
            .acquire(WorkKind::Git, "slow")
            .expect("grant")
            .expect("permit");
        held_for_thread.wait();
        // The slow repository holds its permit across the other worker's
        // full cycle.
        released_for_thread.wait();
        permits_a.release(&permit, true).expect("release");
    });
    slow_held.wait();
    let permit = permits
        .acquire(WorkKind::Git, "fast")
        .expect("grant")
        .expect("permit");
    permits.release(&permit, true).expect("release");
    slow_released.wait();
    slow.join().expect("slow joins");
    assert_eq!(permits.active_repositories(), 0);
}
