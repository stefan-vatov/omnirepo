//! Interpret the owner decision on automated migration.
//!
//! The owner evidence (the .26/.27 outcomes) is read as exactly ONE
//! branch: either automated migration is DECLINED (the current decision
//! for the first constitutional release) or it is NOT selected.  No
//! implementation begins without selection, and the public surface stays
//! migration-free — the guard refuses any command that would smuggle a
//! migration path under another name.

#![allow(dead_code)]

#[cfg(test)]
mod migration_decision_tests;

use std::{error::Error, fmt};

/// The interpreted owner decision.  Exactly one branch is recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MigrationDecision {
    /// Automated migration is declined for the named scope, with the
    /// owner's reason.  No artifact, agent, or command may exist.
    Declined { scope: String, reason: String },
    /// The owner has not selected automated migration; no implementation
    /// begins.
    NotSelected,
}

/// Interpretation failures.
#[derive(Debug)]
pub enum DecisionError {
    AmbiguousEvidence { reason: String },
}

impl fmt::Display for DecisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousEvidence { reason } => {
                write!(formatter, "the owner evidence is ambiguous: {reason}")
            }
        }
    }
}
impl Error for DecisionError {}

/// Interpret the owner decision from the evidence text.
///
/// The approved-decline evidence says "no automated migration artifact,
/// migration agent, or migrate command" and "never migrate ...
/// implicitly" for the named scope; that is the declined branch.  Any
/// other evidence (including silence) is not-selected: no implementation
/// begins.
pub fn interpret_owner_decision(evidence: &str) -> MigrationDecision {
    let declined_markers = [
        "no automated migration artifact",
        "no migrate command",
        "never migrate",
    ];
    let declined = declined_markers
        .iter()
        .filter(|marker| evidence.contains(**marker))
        .count();
    if declined >= 2 {
        MigrationDecision::Declined {
            scope: "first constitutional release".to_owned(),
            reason: "the owner approved an explicit decline: breaking releases ship explicit \
                     release-bound migration guidance instead of automated migration"
                .to_owned(),
        }
    } else {
        MigrationDecision::NotSelected
    }
}

/// Guard the public surface: no command may carry a migration path —
/// neither an explicit `migrate` nor a hidden variant.
pub fn assert_migration_free_surface(commands: &[String]) -> bool {
    !commands.iter().any(|command| {
        command == "migrate" || command.contains("migrat") || command.contains("migration")
    })
}
