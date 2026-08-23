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

use crate::platform::{AuthorityRoot, ReadOnly, RelativePath, SourceSnapshotRoot};
use crate::source::{
    RevisionId, SourceDeclaration, SourceId, parse_declarations, read_revision_file,
};
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
    let file_label = file.display();
    let bytes = read_revision_file(
        root.display_path().as_path(),
        revision.as_str(),
        &file_label,
    )
    .map_err(|error| {
        format!(
            "source {} declaration {file_label} is unreadable at revision {}: {error}",
            source.as_str(),
            revision.as_str()
        )
    })?;
    let content = String::from_utf8(bytes).map_err(|error| {
        format!(
            "source {} declaration {file_label} is not UTF-8 at revision {}: {error}",
            source.as_str(),
            revision.as_str()
        )
    })?;
    parse_declarations(source, revision, &[(&file_label, content)])
        .map_err(|error| error.to_string())
}
