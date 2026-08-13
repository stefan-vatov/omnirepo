//! Authority-typed source declaration reading (the source snapshot seam).
//!
//! The composition side resolves each owner-selected declaration file inside
//! the pinned snapshot through the typed read-only source authority: only
//! `SourceSnapshotRoot` + `ReadOnly` handles, no-follow relative resolution,
//! regular-file targets only.  The bytes go to the pure parser in `source`;
//! no destination access or content extraction happens here.

#![allow(dead_code)]

use crate::platform::{AuthorityRoot, ObjectClass, ReadOnly, RelativePath, SourceSnapshotRoot};
use crate::source::{
    DeclarationsError, RevisionId, SourceDeclaration, SourceId, parse_declarations,
};
use std::io::Read;

/// Read and parse the owner-selected declaration files inside the pinned
/// snapshot.  Files are resolved no-follow through the authority root; an
/// absent, aliased, linked, non-regular, or unreadable target fails typed.
pub fn read_source_declarations(
    root: &AuthorityRoot<SourceSnapshotRoot, ReadOnly>,
    source: &SourceId,
    revision: &RevisionId,
    files: &[RelativePath],
) -> Result<Vec<SourceDeclaration>, DeclarationsError> {
    let mut contents = Vec::with_capacity(files.len());
    for file in files {
        let file_label: String = file.display();
        let target = root
            .resolve_read(file, ObjectClass::RegularFile)
            .map_err(|error| DeclarationsError::MalformedRecord {
                source: source.clone(),
                file: file.display(),
                entry: 0,
                reason: format!("unreadable through the source authority: {error}"),
            })?;
        let mut handle =
            target
                .try_clone_file()
                .map_err(|error| DeclarationsError::MalformedRecord {
                    source: source.clone(),
                    file: file.display(),
                    entry: 0,
                    reason: format!("cannot clone the read handle: {error}"),
                })?;
        let mut content = String::new();
        handle.read_to_string(&mut content).map_err(|error| {
            DeclarationsError::MalformedRecord {
                source: source.clone(),
                file: file.display(),
                entry: 0,
                reason: format!("cannot read the declaration content: {error}"),
            }
        })?;
        contents.push((file_label, content));
    }
    let references: Vec<(&str, String)> = contents
        .iter()
        .map(|(label, content)| (label.as_str(), content.clone()))
        .collect();
    parse_declarations(source, revision, &references)
}

#[cfg(test)]
mod source_catalog_tests;
