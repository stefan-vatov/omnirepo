//! The runtime source catalog built from the machine authority.
//!
//! Each machine source is recorded in declared order. Local sources pin their
//! clean `main`; remote sources use an immutable fetched snapshot. Doctor
//! inspects only an already materialized remote snapshot, while sync fetches
//! and materializes before planning. An unavailable higher-priority source is
//! retained as an explicit failure and never promotes a lower source.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_catalog_tests;

use crate::configuration::{MachineConfiguration, SourceLocation};
use crate::platform::{AuthorityRoot, ReadOnly, SourceSnapshotRoot};
use crate::source::{
    AcquireConfig, CatalogState, RevisionId, SourceCatalog, SourceId, acquire,
    inspect_cached_remote, remote_snapshot_path,
};
use std::{collections::HashMap, error::Error, fmt, path::Path, path::PathBuf};

/// One exact source revision and the Git root that owns its objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedSource {
    pub root: PathBuf,
    pub revision: RevisionId,
}

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
/// Local sources pin their clean `main`. Remote sources are complete only when
/// a previously fetched immutable snapshot can be inspected without effects.
pub fn build_runtime_catalog(
    config: &MachineConfiguration,
) -> Result<SourceCatalog, CatalogBuildError> {
    build_catalog(config, false)
}

/// Build the sync catalog, fetching and materializing each remote source
/// before its immutable revision enters planning.
pub fn build_sync_runtime_catalog(
    config: &MachineConfiguration,
) -> Result<SourceCatalog, CatalogBuildError> {
    build_catalog(config, true)
}

fn build_catalog(
    config: &MachineConfiguration,
    fetch_remotes: bool,
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
            SourceLocation::Remote(_) => {
                let state = config
                    .cache_root()
                    .ok_or_else(|| "remote source has no machine cache root".to_owned())
                    .and_then(|cache_root| {
                        remote_source_state(
                            &source_id,
                            source,
                            Path::new(cache_root.as_str()),
                            fetch_remotes,
                        )
                    });
                match state {
                    Ok(state) => state,
                    Err(reason) => CatalogState::Unavailable {
                        source: source_id.clone(),
                        reason,
                    },
                }
            }
        };
        catalog
            .record(state)
            .map_err(|_| CatalogBuildError::SourceUnavailable)?;
    }
    Ok(catalog)
}

/// Resolve the exact Git root and revision for every complete catalog entry.
pub fn materialized_sources(
    config: &MachineConfiguration,
    catalog: &SourceCatalog,
) -> Result<HashMap<String, MaterializedSource>, CatalogBuildError> {
    let mut sources = HashMap::new();
    for state in catalog.entries() {
        let CatalogState::Complete { source, revision } = state else {
            continue;
        };
        let reference = config
            .sources()
            .iter()
            .find(|reference| reference.id().as_str() == source.as_str())
            .ok_or(CatalogBuildError::SourceUnavailable)?;
        let path = match reference.location() {
            SourceLocation::Local(path) => PathBuf::from(path.as_str()),
            SourceLocation::Remote(_) => {
                let cache_root = config
                    .cache_root()
                    .ok_or(CatalogBuildError::SourceUnavailable)?;
                remote_snapshot_path(
                    Path::new(cache_root.as_str()),
                    source.as_str(),
                    revision.as_str(),
                )
            }
        };
        AuthorityRoot::<SourceSnapshotRoot, ReadOnly>::open(&path)
            .map_err(|_| CatalogBuildError::SourceUnavailable)?;
        sources.insert(
            source.as_str().to_owned(),
            MaterializedSource {
                root: path,
                revision: revision.clone(),
            },
        );
    }
    Ok(sources)
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

fn remote_source_state(
    source: &SourceId,
    reference: &crate::configuration::SourceReference,
    cache_root: &Path,
    fetch: bool,
) -> Result<CatalogState, String> {
    let config = AcquireConfig::new(cache_root);
    let snapshot = if fetch {
        acquire(reference, &config)
    } else {
        inspect_cached_remote(reference, &config)
    }
    .map_err(|error| error.to_string())?;
    let root = Path::new(snapshot.cache().as_str());
    AuthorityRoot::<SourceSnapshotRoot, ReadOnly>::open(root).map_err(|error| error.to_string())?;
    Ok(CatalogState::Complete {
        source: source.clone(),
        revision: snapshot.revision().clone(),
    })
}
