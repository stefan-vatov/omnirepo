//! A pure state machine for one managed-file replacement.
//!
//! This module records the protocol.  It does not open files, create
//! directories, write bytes, rename paths, or promise a stronger durability
//! boundary than the managed-content and fleet-lifecycle contracts provide.
//! The filesystem implementation can use these states as its journal-facing
//! vocabulary and can reject an operation before it performs an invalid step.
//! Path checks here are lexical and portable only.  Canonical no-follow
//! filesystem identity, containment, mount, alias, and object checks remain
//! the authority-adapter boundary owned by the .8 workstream.
//!
//! The module is split into focused submodules: protocol states ([`state`]),
//! the transaction plan ([`plan`]), candidates and artifacts
//! ([`candidates`], [`artifact`]), recovery observations ([`recovery`]),
//! typed errors ([`errors`]), and the transaction machine ([`transaction`]).

mod artifact;
mod candidates;
mod errors;
mod plan;
mod recovery;
mod state;
#[allow(clippy::module_inception)]
mod transaction;

pub(crate) use artifact::TempArtifact;
pub(crate) use candidates::{CandidateError, TempCandidate, validate_relative_path};
pub(crate) use errors::{ProofError, TransactionError};
pub(crate) use plan::{ParentDirectories, PlanError, TransactionPlan};

pub(crate) use recovery::{
    CleanupDisposition, CleanupResult, FailureEvidence, FailureKind, MetadataResult,
    RecoveryBinding, RecoveryDurabilityProof, RecoveryNextAction, RecoveryObservation,
    RecoveryResult,
};
#[cfg(test)]
pub(crate) use state::ContentVisibility;

#[cfg(test)]
mod transaction_a_tests;
#[cfg(test)]
mod transaction_b_tests;
pub(crate) use state::{Comparison, JournalCheckpoint, TransactionState};
#[cfg(test)]
pub(crate) use transaction::Transaction;
