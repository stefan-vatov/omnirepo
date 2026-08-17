//! Private platform capabilities shared by product lifecycle code.

#![allow(dead_code)]

mod authority;

pub(crate) use authority::{
    AgentWorkingDirectoryRoot, AuthorityIdentity, AuthorityRoot, DestinationRepositoryRoot,
    FilesystemIdentity, GitWorkingDirectoryRoot, Mutate, MutationIntent, ObjectClass,
    ObjectIdentity, PathError, ReadOnly, RelativePath, RunRecordRoot, SourceSnapshotRoot,
    open_mutation_root, resolve_mutation, sync_directory, sync_file,
};

#[cfg(test)]
pub(crate) use authority::open_read_root;

#[cfg(test)]
pub(crate) use authority::{test_creation_mode, test_durability_phase, test_reset_observations};
