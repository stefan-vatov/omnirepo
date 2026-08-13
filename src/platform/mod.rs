//! Private platform capabilities shared by product lifecycle code.

#![allow(dead_code)]

mod authority;

pub(crate) use authority::{
    DestinationRepositoryRoot, MutationIntent, PathError, RelativePath, RunRecordRoot,
    open_mutation_root, sync_directory, sync_file,
};

// Read-target authority seams consumed by test fixtures today and by the
// repository-domain readers in later slices.
#[cfg(test)]
pub(crate) use authority::{ObjectClass, open_read_root};

#[cfg(test)]
pub(crate) use authority::{test_creation_mode, test_durability_phase, test_reset_observations};
