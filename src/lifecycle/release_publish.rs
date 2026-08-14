//! Publish prequalified artifacts to selected channels.
//!
//! Publication is fail-closed: an artifact reaches a channel only when
//! every normative gate passed, the provenance verified, and the
//! canonical release tag is valid.  A non-public candidate is recorded
//! for internal distribution only; the public channel additionally
//! requires the explicit promotion gate.  Publication never touches the
//! main branch (the unsafe main-push tagging quarantine).

#![allow(dead_code)]

#[cfg(test)]
mod release_publish_tests;

use crate::lifecycle::release_gates::GateRun;
use crate::lifecycle::release_manifest::CandidateManifest;
use crate::lifecycle::release_tag::TagValidation;
use std::{error::Error, fmt};

/// The selected channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    NonPublic,
    Public,
}

/// One publication request.
#[derive(Clone, Debug)]
pub struct PublishRequest {
    pub channel: Channel,
    pub manifest: CandidateManifest,
    pub gates: Vec<GateRun>,
    pub provenance_ok: bool,
    pub tag: TagValidation,
    pub promotion: bool,
}

/// The publication outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishOutcome {
    pub published: bool,
    pub public_channel: bool,
    pub main_branch_touched: bool,
}

/// Publication failures.
#[derive(Debug)]
pub enum PublishError {
    NotPrequalified { reason: String },
    PromotionRequired,
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrequalified { reason } => {
                write!(formatter, "the candidate is not prequalified: {reason}")
            }
            Self::PromotionRequired => {
                write!(
                    formatter,
                    "the public channel requires the explicit promotion gate"
                )
            }
        }
    }
}
impl Error for PublishError {}

/// Publish the candidate to the selected channel, fail-closed.
pub fn publish_prequalified(request: &PublishRequest) -> Result<PublishOutcome, PublishError> {
    let gate_failures = request
        .gates
        .iter()
        .filter(|gate| !gate.passed)
        .map(|gate| gate.name.clone())
        .collect::<Vec<_>>();
    if !gate_failures.is_empty() {
        return Err(PublishError::NotPrequalified {
            reason: format!("failing gates: {}", gate_failures.join(", ")),
        });
    }
    if !request.provenance_ok {
        return Err(PublishError::NotPrequalified {
            reason: "the candidate provenance did not verify".to_owned(),
        });
    }
    if !request.tag.annotated {
        return Err(PublishError::NotPrequalified {
            reason: "the canonical release tag is not annotated".to_owned(),
        });
    }
    if request.manifest.identity.source_commit != request.tag.commit {
        return Err(PublishError::NotPrequalified {
            reason: "the manifest commit does not match the release tag".to_owned(),
        });
    }
    let public_channel = request.channel == Channel::Public;
    if public_channel && !request.promotion {
        return Err(PublishError::PromotionRequired);
    }
    Ok(PublishOutcome {
        published: true,
        public_channel,
        // The unsafe main-push tagging quarantine: publication records
        // the candidate only and never touches the main branch.
        main_branch_touched: false,
    })
}
