//! Focused proof for nested child-work permits.

#![allow(dead_code, unused_imports)]

use super::{ChildPermit, NestedPermits, PermitError, WorkKind};

fn ledger() -> NestedPermits {
    NestedPermits::new(
        4,
        &[
            (WorkKind::Verify, 2),
            (WorkKind::Git, 1),
            (WorkKind::Source, 2),
            (WorkKind::Agent, 1),
        ],
    )
    .expect("ledger")
}

#[test]
fn global_and_kind_limits_are_respected() {
    let permits = ledger();
    // Global limit 4 (the per-kind budgets allow this spread: two verify,
    // one git, one source).
    let held: Vec<ChildPermit> = [
        (WorkKind::Verify, "a"),
        (WorkKind::Verify, "b"),
        (WorkKind::Git, "c"),
        (WorkKind::Source, "d"),
    ]
    .iter()
    .map(|(kind, repo)| {
        permits
            .acquire(*kind, *repo)
            .expect("grant")
            .expect("permit")
    })
    .collect();
    assert_eq!(permits.active_repositories(), 4);
    assert!(
        permits
            .acquire(WorkKind::Verify, "e")
            .expect("grant")
            .is_none()
    );
    for permit in held {
        permits.release(&permit, true).expect("release");
    }
    // Per-kind limit: two verify permits, a git permit is refused.
    let v1 = permits
        .acquire(WorkKind::Verify, "a")
        .expect("grant")
        .expect("v1");
    let v2 = permits
        .acquire(WorkKind::Verify, "b")
        .expect("grant")
        .expect("v2");
    assert!(
        permits
            .acquire(WorkKind::Verify, "c")
            .expect("grant")
            .is_none()
    );
    // Git has its own budget: one git permit fits.
    let g1 = permits
        .acquire(WorkKind::Git, "c")
        .expect("grant")
        .expect("g1");
    assert!(
        permits
            .acquire(WorkKind::Git, "d")
            .expect("grant")
            .is_none()
    );
    permits.release(&v1, true).expect("release");
    permits.release(&v2, true).expect("release");
    permits.release(&g1, true).expect("release");
}

#[test]
fn one_repository_never_overlaps_stages() {
    let permits = ledger();
    let verify = permits
        .acquire(WorkKind::Verify, "a")
        .expect("grant")
        .expect("verify");
    // The same repository cannot take another stage while it holds one.
    assert!(
        permits
            .acquire(WorkKind::Git, "a")
            .expect("grant")
            .is_none()
    );
    assert!(
        permits
            .acquire(WorkKind::Agent, "a")
            .expect("grant")
            .is_none()
    );
    permits.release(&verify, true).expect("release");
    // After release the repository can take the next stage.
    let git = permits
        .acquire(WorkKind::Git, "a")
        .expect("grant")
        .expect("git");
    permits.release(&git, true).expect("release");
}

#[test]
fn release_requires_descendant_termination() {
    let permits = ledger();
    let agent = permits
        .acquire(WorkKind::Agent, "a")
        .expect("grant")
        .expect("agent");
    // A live descendant keeps the permit held.
    let error = permits
        .release(&agent, false)
        .expect_err("descendants live");
    assert!(
        matches!(error, PermitError::DescendantsActive { .. }),
        "{error}"
    );
    // The permit is still held, so no new stage can overlap.
    assert!(
        permits
            .acquire(WorkKind::Verify, "a")
            .expect("grant")
            .is_none()
    );
    // Termination confirmation releases it.
    permits.release(&agent, true).expect("release");
    assert!(
        permits
            .acquire(WorkKind::Verify, "a")
            .expect("grant")
            .is_some()
    );
}

#[test]
fn cancellation_stops_new_acquisition_and_release_checks_termination() {
    let permits = ledger();
    let agent = permits
        .acquire(WorkKind::Agent, "a")
        .expect("grant")
        .expect("agent");
    permits.cancel();
    let error = permits
        .acquire(WorkKind::Verify, "b")
        .expect_err("cancelled");
    assert!(matches!(error, PermitError::RunCancelled), "{error}");
    // Cancellation releases the permit only after descendant termination.
    assert!(permits.release(&agent, false).is_err());
    permits.release(&agent, true).expect("release");
}
