//! The runtime source catalog built from the machine authority.
//!
//! Each machine source is recorded in declared order: a local source
//! records Complete after its typed read-only source root opens and its
//! revision pins; a remote source records Unavailable with a typed
//! reason until materialization is available.  An unavailable
//! higher-priority source is retained as an explicit failure and never
//! promotes a lower source.  The build scans nothing beyond the declared
//! sources and produces no source or destination effect.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_catalog_tests;

use crate::configuration::{MachineConfiguration, SourceLocation};
use crate::platform::{AuthorityRoot, ReadOnly, SourceSnapshotRoot};
use crate::source::{AcquireConfig, CatalogState, SourceCatalog, SourceId, acquire};
use std::{error::Error, fmt, path::Path};

/// Catalog build failures (defensive; availability is per source).
#[derive(Debug)]
pub enum CatalogBuildError {
    SourceUnavailable,
}

impl fmt::Display for CatalogBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceUnavailable => {
                write!(
                    formatter,
                    "the source is unavailable; the typed reason is per source"
                )
            }
        }
    }
}
impl Error for CatalogBuildError {}

/// Build the runtime source catalog from the machine authority.
///
/// Declared order is preserved; every source records exactly one state.
/// A local source is Complete only when its typed read-only root opens
/// and its revision pins; anything else (including remote sources before
/// materialization) is Unavailable with a typed reason.  No ambient scan
/// and no source or destination effect.
pub fn build_runtime_catalog(
    config: &MachineConfiguration,
) -> Result<SourceCatalog, CatalogBuildError> {
    let mut catalog = SourceCatalog::new();
    for source in config.sources() {
        let source_id = SourceId::new(source.id().as_str())
            .map_err(|_| CatalogBuildError::SourceUnavailable)?;
        let state = match source.location() {
            SourceLocation::Local(path) => {
                match local_source_state(&source_id, source, Path::new(path.as_str())) {
                    Ok(state) => state,
                    Err(reason) => CatalogState::Unavailable {
                        source: source_id.clone(),
                        reason,
                    },
                }
            }
            SourceLocation::Remote(_) => CatalogState::Unavailable {
                source: source_id.clone(),
                reason: "remote materialization is not available for this run".to_owned(),
            },
        };
        catalog
            .record(state)
            .map_err(|_| CatalogBuildError::SourceUnavailable)?;
    }
    Ok(catalog)
}

/// One local source: the typed read-only root must open (no-follow) and
/// the revision must pin (the canonical HEAD of the clean worktree).
fn local_source_state(
    source: &SourceId,
    reference: &crate::configuration::SourceReference,
    path: &Path,
) -> Result<CatalogState, String> {
    let root = AuthorityRoot::<SourceSnapshotRoot, ReadOnly>::open(path)
        .map_err(|error| error.to_string())?;
    let _ = root;
    let snapshot =
        acquire(reference, &AcquireConfig::new(path)).map_err(|error| error.to_string())?;
    Ok(CatalogState::Complete {
        source: source.clone(),
        revision: snapshot.revision().clone(),
    })
}
