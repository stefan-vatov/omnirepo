//! Focused proof for bounded admission queues and permits.

#![allow(dead_code, unused_imports)]

use super::{FleetPermits, PermitError};

#[test]
fn active_count_never_exceeds_the_limit() {
    let permits = FleetPermits::new(2).expect("ledger");
    for id in ["a", "b", "c", "d"] {
        permits.enqueue(id);
    }
    assert_eq!(permits.queued(), 4);
    let first = permits.grant_next().expect("grant").expect("a");
    let second = permits.grant_next().expect("grant").expect("b");
    assert_eq!(permits.active(), 2);
    assert_eq!(permits.queued(), 2);
    // The limit is saturated: no further grant until a permit drops.
    assert!(permits.grant_next().expect("grant").is_none());
    drop(first);
    assert_eq!(permits.active(), 1);
    let third = permits.grant_next().expect("grant").expect("c");
    assert_eq!(permits.active(), 2);
    drop(second);
    drop(third);
    assert_eq!(permits.active(), 0);
    let fourth = permits.grant_next().expect("grant").expect("d");
    assert_eq!(permits.active(), 1);
    drop(fourth);
}

#[test]
fn zero_limit_is_refused_typed() {
    let error = FleetPermits::new(0).expect_err("zero limit");
    assert!(matches!(error, PermitError::Limit { .. }), "{error}");
}

#[test]
fn cancellation_stops_new_admission() {
    let permits = FleetPermits::new(2).expect("ledger");
    permits.enqueue("a");
    permits.cancel();
    let error = permits.grant_next().expect_err("cancelled");
    assert!(matches!(error, PermitError::RunCancelled), "{error}");
}

#[test]
fn writer_failure_stops_new_admission() {
    let permits = FleetPermits::new(2).expect("ledger");
    permits.enqueue("a");
    permits.mark_writer_unhealthy();
    let error = permits.grant_next().expect_err("unhealthy");
    assert!(matches!(error, PermitError::WriterUnhealthy), "{error}");
}

#[test]
fn queue_order_does_not_affect_accounting() {
    let permits = FleetPermits::new(1).expect("ledger");
    // Enqueue in reverse order; every repository still reaches its own
    // terminal slot exactly once.
    for id in ["z", "y", "x"] {
        permits.enqueue(id);
    }
    let mut granted = Vec::new();
    while let Some(permit) = permits.grant_next().expect("grant") {
        granted.push(permit.repository.clone());
        drop(permit);
    }
    assert_eq!(granted, vec!["z", "y", "x"]);
    assert_eq!(permits.active(), 0);
    assert_eq!(permits.queued(), 0);
}
