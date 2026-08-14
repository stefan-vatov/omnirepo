//! Authority-root and no-follow filesystem primitives.
//!
//! This module is deliberately independent from the legacy path helpers.  A
//! caller must first open a typed authority root and then resolve a validated
//! [`RelativePath`] through that root.  The returned handle owns the checked
//! object, so a later caller cannot replace it with a path from another root.
//!
//! The module is split into focused submodules: identities and errors
//! ([`identity`]), validated paths ([`paths`]), roots and targets ([`roots`]),
//! and the platform backend ([`backend`]).

mod backend;
mod identity;
mod paths;
mod roots;

pub use identity::{
    AuthorityAdapterKind, AuthorityIdentity, FilesystemIdentity, FilesystemKind, MutationIntent,
    ObjectClass, ObjectIdentity, PathError,
};
pub use paths::{AbsolutePath, RelativePath};
pub use roots::{
    AgentWorkingDirectoryRoot, AuthorityRoot, DestinationRepositoryRoot, GitWorkingDirectoryRoot,
    Mutate, MutationAllowed, MutationTarget, ReadOnly, ReadTarget, RunRecordRoot,
    SourceSnapshotRoot, open_mutation_root,
};

#[cfg(test)]
pub use roots::{
    AuthorityAcceptance, AuthorityAdapter, AuthorityRegistry, MachineConfigRoot, open_read_root,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static TEST_CREATION_MODE: Cell<u32> = const { Cell::new(0) };
    static TEST_DURABILITY_PHASE: Cell<u8> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn test_reset_observations() {
    TEST_CREATION_MODE.with(|value| value.set(0));
    TEST_DURABILITY_PHASE.with(|value| value.set(0));
}

#[cfg(test)]
pub(crate) fn test_creation_mode() -> u32 {
    TEST_CREATION_MODE.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn test_durability_phase() -> u8 {
    TEST_DURABILITY_PHASE.with(Cell::get)
}

#[cfg(all(test, unix))]
fn observe_creation_mode(file: &std::fs::File) {
    use std::os::unix::fs::PermissionsExt;

    let mode = file
        .metadata()
        .expect("test mode observation reads the new file")
        .permissions()
        .mode()
        & 0o777;
    TEST_CREATION_MODE.with(|value| value.set(mode));
}

#[cfg(any(not(test), not(unix)))]
fn observe_creation_mode(_file: &std::fs::File) {}

#[cfg(test)]
fn mark_file_sync_complete() {
    TEST_DURABILITY_PHASE.with(|value| value.set(1));
}

#[cfg(not(test))]
fn mark_file_sync_complete() {}

#[cfg(test)]
fn mark_directory_sync_complete() {
    TEST_DURABILITY_PHASE.with(|value| {
        assert_eq!(value.get(), 1, "directory sync must follow file sync");
        value.set(2);
    });
}

#[cfg(not(test))]
fn mark_directory_sync_complete() {}

/// Synchronize a newly created file through the platform-owned backend.
pub(crate) fn sync_file(file: &std::fs::File, path: &str) -> Result<(), PathError> {
    backend::sync_file(file, path)
}

/// Synchronize a containing directory through the platform-owned backend.
pub(crate) fn sync_directory(directory: &std::fs::File, path: &str) -> Result<(), PathError> {
    backend::sync_directory(directory, path)
}
