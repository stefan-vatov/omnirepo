//! Pure repository-state values shared by planning, verification, repair, Git,
//! journaling, and recovery.
//!
//! This module intentionally has no filesystem, Git, process, network, or
//! clock effects.  It records observed facts, frozen operation witnesses, and
//! the exact current-run delta as immutable, deterministically ordered values.
//!
//! The module is split into focused submodules: the domain error and text
//! identities ([`domain_error`]), path and filesystem identities
//! ([`identities`]), managed targets and index/worktree state ([`targets`]),
//! Git facts ([`git_facts`]), repository facts and snapshots ([`facts`]),
//! the authorized delta ([`delta`]), causation proofs ([`causation`]), and
//! the canonical representation ([`canonical`]).

mod canonical;
mod causation;
mod delta;
mod domain_error;
mod facts;
mod git_facts;
mod identities;
mod targets;

pub(crate) use canonical::CanonicalRepresentation;
pub(crate) use causation::{CausationBasis, CausationRelation};

#[cfg(test)]
pub(crate) use canonical::CANONICAL_REPOSITORY_STATE_VERSION;
#[cfg(test)]
pub(crate) use causation::{
    BaselineIdentityProof, CausationAssessment, DirectCausationProof, InferredCausation,
    ManagedPathFailureProof, ObservedFact, OwnerDecision,
};
pub(crate) use delta::{AuthorizedChange, AuthorizedDelta};
pub(crate) use domain_error::{CheckWitness, RefName, RepositoryId, RevisionId};
pub(crate) use domain_error::{DomainError, validate_text};
pub(crate) use facts::GitRepositoryState;
pub(crate) use facts::{FrozenWitnesses, RepositoryFacts, RepositorySnapshot};
pub(crate) use git_facts::{GitFacts, HeadState, UpstreamState};

pub(crate) use identities::{
    AuthorityIdentity, FilesystemClass, FilesystemIdentity, ManagedSectionId, ObjectIdentity,
    RelativePath, RenamePaths, RepositoryRoot,
};
pub(crate) use targets::{
    DirtyProvenance, EntryKind, FileIdentity, IndexEntry, IndexState, ManagedOwnership,
    ManagedTargetIdentity, TargetChange, WorktreeEntry, WorktreeState,
};

#[cfg(test)]
mod state_a_tests;
#[cfg(test)]
mod state_b_tests;
