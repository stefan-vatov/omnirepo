#![allow(dead_code)]

mod capture;
mod git_index;
mod manifest;
mod operation_commit;
mod operation_tree;
mod policy;
mod policy_loader;
mod revalidate;
mod state;

pub(crate) use capture::capture_state;
pub(crate) use git_index::IsolatedIndex;
pub(crate) use operation_commit::{CommitError, RecordedCommit, create_commit};
pub(crate) use state::{
    GitRepositoryState, HeadState, RefName, RepositoryId, RevisionId, UpstreamState,
};

#[cfg(test)]
pub(crate) use git_index::prepare_index;

#[cfg(test)]
pub(crate) use manifest::{PlannedOperation, build_authorized_delta};

#[cfg(test)]
pub(crate) use state::AuthorizedDelta;

#[cfg(test)]
pub(crate) use state::{
    AuthorityIdentity, CheckWitness, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, GitFacts, IndexState, ManagedTargetIdentity, ObjectIdentity, RelativePath,
    RepositoryFacts, RepositoryRoot, RepositorySnapshot, WorktreeState,
};

#[cfg(test)]
mod capture_tests;

#[cfg(test)]
mod git_index_tests;

#[cfg(test)]
mod operation_commit_tests;

#[cfg(test)]
mod operation_tree_tests;

#[cfg(test)]
mod manifest_tests;

#[cfg(test)]
mod policy_loader_tests;

#[cfg(test)]
mod policy_tests;

#[cfg(test)]
mod state_tests;
