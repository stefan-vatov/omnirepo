#![allow(unreachable_patterns)]

//! Shared, visible capability classification for authority fixture suites.

use std::path::Path;

use crate::platform::authority::{
    AuthorityRoot, DestinationRepositoryRoot, FilesystemKind, ReadOnly,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Classification {
    Exercised(FilesystemKind),
    Unsupported(String),
}

pub(crate) fn classify(path: &Path) -> Classification {
    match AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(path) {
        Ok(root) => Classification::Exercised(root.identity().filesystem().kind()),
        Err(crate::platform::authority::PathError::UnsupportedFilesystem { kind, .. }) => {
            Classification::Unsupported(kind)
        }
        Err(error) => panic!("authority capability probe failed before classification: {error}"),
    }
}

pub(crate) fn report(path: &Path) -> Classification {
    let classification = classify(path);
    match &classification {
        Classification::Exercised(FilesystemKind::LinuxExtFamily) => {
            eprintln!("authority-capability: exercised-supported=linux-ext-family")
        }
        Classification::Exercised(FilesystemKind::MacOsApfs) => {
            eprintln!("authority-capability: exercised-supported=macos-apfs")
        }
        Classification::Exercised(other) => {
            eprintln!("authority-capability: exercised-supported=unexpected-{other:?}")
        }
        Classification::Unsupported(kind) => {
            eprintln!("authority-capability: unsupported-filesystem={kind}")
        }
    }
    classification
}
