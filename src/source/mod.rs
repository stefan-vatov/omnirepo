#![allow(dead_code)]

mod acquisition;
mod declarations;
mod publish;
mod snapshot;

pub(crate) use declarations::{DeclarationsError, SourceDeclaration, parse_declarations};
pub(crate) use snapshot::{RevisionId, SourceId};

#[cfg(test)]
pub(crate) use declarations::DECLARATION_VERSION;

#[cfg(test)]
mod acquisition_tests;

#[cfg(test)]
mod declarations_tests;

#[cfg(test)]
mod publish_tests;

#[cfg(test)]
mod snapshot_tests;
