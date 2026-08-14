//! Exhaustive failure-stage to repair-eligibility classification.
//!
//! Only selected sync and verification failure classes may proceed to
//! bounded repair; shared authority, Git, journal, unrelated, and
//! uncertain classes are terminal.  The classification is pure and every
//! class carries a stable explanation.

#![allow(dead_code)]

use std::{error::Error, fmt};

#[cfg(test)]
mod repair_classify_tests;

/// The failure classes observed across the run stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureClass {
    SyncDrift,
    VerificationFailed,
    RepairAttemptFailed,
    MachineAuthorityInvalid,
    SourceAuthorityInvalid,
    GitDeliveryFailed,
    JournalFailure,
    Unrelated,
    Uncertain,
}

/// The repair eligibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Eligibility {
    /// The class may proceed to bounded repair.
    Repairable { attempts: u8 },
    /// The class is terminal: no repair may proceed.
    Terminal,
}

/// One typed classification with its stable explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairClassification {
    pub class: FailureClass,
    pub eligibility: Eligibility,
    pub explanation: String,
}

/// Classification failures (defensive; the classifier is total).
#[derive(Debug)]
pub enum ClassifyError {
    Unknown { class: String },
}

impl fmt::Display for ClassifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { class } => write!(formatter, "unknown failure class {class:?}"),
        }
    }
}
impl Error for ClassifyError {}

/// Classify a failure class into its repair eligibility.  Pure: no I/O, no
/// state.
pub fn classify_failure(class: FailureClass) -> RepairClassification {
    match class {
        FailureClass::SyncDrift => RepairClassification {
            class,
            eligibility: Eligibility::Repairable { attempts: 1 },
            explanation: "sync-induced managed drift is causally repairable".to_owned(),
        },
        FailureClass::VerificationFailed => RepairClassification {
            class,
            eligibility: Eligibility::Repairable { attempts: 1 },
            explanation: "a failed configured check after sync is causally repairable".to_owned(),
        },
        FailureClass::RepairAttemptFailed => RepairClassification {
            class,
            eligibility: Eligibility::Repairable { attempts: 1 },
            explanation: "a failed repair attempt is retriable within its budget".to_owned(),
        },
        FailureClass::MachineAuthorityInvalid => RepairClassification {
            class,
            eligibility: Eligibility::Terminal,
            explanation: "machine authority is shared and invalid; never repairable".to_owned(),
        },
        FailureClass::SourceAuthorityInvalid => RepairClassification {
            class,
            eligibility: Eligibility::Terminal,
            explanation: "source authority is shared; never repairable by a destination".to_owned(),
        },
        FailureClass::GitDeliveryFailed => RepairClassification {
            class,
            eligibility: Eligibility::Terminal,
            explanation: "Git delivery is owner-governed; the class is terminal".to_owned(),
        },
        FailureClass::JournalFailure => RepairClassification {
            class,
            eligibility: Eligibility::Terminal,
            explanation: "journal failures are shared and terminal".to_owned(),
        },
        FailureClass::Unrelated => RepairClassification {
            class,
            eligibility: Eligibility::Terminal,
            explanation: "unrelated repository health is outside repair causation".to_owned(),
        },
        FailureClass::Uncertain => RepairClassification {
            class,
            eligibility: Eligibility::Terminal,
            explanation: "uncertain causation never proceeds to repair".to_owned(),
        },
    }
}
