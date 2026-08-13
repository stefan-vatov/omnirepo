#![allow(dead_code)]

mod acquisition;
mod catalog_state;
mod declarations;
mod extraction;
mod item_resolution;
mod publish;
mod snapshot;

pub(crate) use declarations::{DeclarationsError, SourceDeclaration, parse_declarations};
pub(crate) use extraction::{
    ExtractedPayload, ExtractionError, PayloadKind, extract_payload, validate_locator,
};
pub(crate) use snapshot::{RevisionId, SourceId};

pub(crate) use catalog_state::{CatalogState, SourceCatalog};
pub(crate) use item_resolution::{ItemDeclaration, ResolvedItem, resolve_items};

#[cfg(test)]
pub(crate) use catalog_state::{CatalogError, PlanningImpact, plan_impact};

pub(crate) use item_resolution::ItemKind;

#[cfg(test)]
pub(crate) use item_resolution::{CollisionKind, LoserRef, ResolutionError};

#[cfg(test)]
pub(crate) use declarations::DECLARATION_VERSION;

#[cfg(test)]
pub(crate) use extraction::content_identity;

#[cfg(test)]
mod acquisition_tests;

#[cfg(test)]
mod catalog_state_tests;

#[cfg(test)]
mod catalog_fixture_tests;

#[cfg(test)]
mod declarations_tests;

#[cfg(test)]
mod extraction_tests;

#[cfg(test)]
mod item_resolution_tests;

#[cfg(test)]
mod publish_tests;

#[cfg(test)]
mod snapshot_tests;
