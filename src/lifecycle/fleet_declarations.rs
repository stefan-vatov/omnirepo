//! Read the pinned source declarations through the typed source root.
//!
//! The canonical declaration file `<source-root>/.omnirepo/source.yaml`
//! is read through the typed read-only SourceSnapshotRoot with no-follow
//! containment and parsed in declared order.  An unsupported version,
//! malformed content, or a missing file fails typed with the source and
//! file named — never silent absence.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_declarations_tests;

use crate::lifecycle::source_catalog::read_source_declarations;
use crate::platform::{AuthorityRoot, ReadOnly, RelativePath, SourceSnapshotRoot};
use crate::source::{RevisionId, SourceDeclaration, SourceId};
use std::path::Path;

/// The canonical declaration file inside each source snapshot.
const DECLARATION_FILE: &str = ".omnirepo/source.yaml";

/// Read and parse the pinned source declarations.
pub fn read_pinned_declarations(
    source: &SourceId,
    revision: &RevisionId,
    source_root: &Path,
) -> Result<Vec<SourceDeclaration>, String> {
    let root = AuthorityRoot::<SourceSnapshotRoot, ReadOnly>::open(source_root)
        .map_err(|error| error.to_string())?;
    let file = RelativePath::parse(DECLARATION_FILE).map_err(|error| error.to_string())?;
    read_source_declarations(&root, source, revision, &[file]).map_err(|error| error.to_string())
}
