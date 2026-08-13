//! Private platform capabilities shared by product lifecycle code.

#![allow(dead_code)]

mod authority;

pub(crate) use authority::{
    MutationIntent, PathError, RelativePath, RunRecordRoot, open_mutation_root, sync_directory,
    sync_file,
};

#[cfg(test)]
pub(crate) use authority::{test_creation_mode, test_durability_phase, test_reset_observations};
