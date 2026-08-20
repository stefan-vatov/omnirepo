//! Focused proof for interpreting the owner decision on automated
//! migration.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::migration_decision::{
    MigrationDecision, assert_migration_free_surface, interpret_owner_decision,
};

const DECLINED_EVIDENCE: &str = "OWNER DECISION — APPROVED 2026-08-13. Breaking releases must ship \
explicit release-bound actionable migration guidance. The first constitutional release deliberately \
provides no automated migration artifact, migration agent, or migrate command. Installation, update, \
configuration loading, setup, validation, and synchronization never migrate configuration or \
destination repositories implicitly. Automated migration may be reconsidered only through a later \
explicit owner decision.";

#[test]
fn the_approved_decline_is_interpreted_as_exactly_one_declined_branch() {
    let decision = interpret_owner_decision(DECLINED_EVIDENCE);
    assert!(matches!(decision, MigrationDecision::Declined { .. }));
    if let MigrationDecision::Declined { scope, reason } = decision {
        assert!(scope.contains("first constitutional release"), "{scope}");
        assert!(!reason.is_empty(), "the decline carries the owner reason");
    }
}

#[test]
fn silent_owner_evidence_selects_no_implementation() {
    let decision = interpret_owner_decision("");
    assert_eq!(decision, MigrationDecision::NotSelected);
}

#[test]
fn the_public_surface_stays_migration_free() {
    // The public command surface of the first constitutional release.
    let commands = ["sync".to_owned(), "setup".to_owned(), "doctor".to_owned()];
    assert!(assert_migration_free_surface(&commands));
}

#[test]
fn a_migrate_command_is_never_admitted() {
    let commands = vec!["sync".to_owned(), "migrate".to_owned()];
    assert!(!assert_migration_free_surface(&commands));
    let hidden = vec!["sync".to_owned(), "migrate-config".to_owned()];
    assert!(!assert_migration_free_surface(&hidden));
}

#[test]
fn the_runtime_command_surface_carries_no_migration_path() {
    // The actual clap surface of the binary: exactly sync, setup, doctor.
    let mut names = crate::configuration::command_surface();
    names.sort();
    assert_eq!(names, vec!["doctor", "setup", "sync"], "{names:?}");
    assert!(assert_migration_free_surface(&names));
}
