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
pub(crate) use git_index::{IsolatedIndex, prepare_index};
pub(crate) use manifest::{PlannedOperation, build_authorized_delta};
pub(crate) use operation_commit::{
    CommitError, RecordedCommit, create_commit, index_matches_parent,
};
pub(crate) use policy::{RepositoryPolicy, VerificationCommand};
pub(crate) use policy_loader::{POLICY_FILE_NAME, PolicyPresence, load_policy};

#[cfg(test)]
pub(crate) use policy_loader::PolicyLoadError;
pub(crate) use state::{
    AuthorityIdentity, EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity,
    FrozenWitnesses, IndexState, ManagedTargetIdentity, ObjectIdentity, RelativePath,
    RepositoryFacts, RepositoryRoot, TargetChange, WorktreeState,
};
pub(crate) use state::{
    GitRepositoryState, HeadState, RefName, RepositoryId, RepositorySnapshot, RevisionId,
    UpstreamState,
};

#[cfg(test)]
pub(crate) use state::AuthorizedDelta;

#[cfg(test)]
pub(crate) use state::{CheckWitness, GitFacts};

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
