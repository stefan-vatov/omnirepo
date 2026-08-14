//! Focused proof for publishing prequalified artifacts to selected
//! channels.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_gates::GateRun;
use crate::lifecycle::release_manifest::{CandidateManifest, manifest_for};
use crate::lifecycle::release_publish::{
    Channel, PublishError, PublishOutcome, PublishRequest, publish_prequalified,
};
use crate::lifecycle::release_tag::TagValidation;

fn manifest() -> CandidateManifest {
    manifest_for(
        "0.9.0",
        "0123456789abcdef0123456789abcdef01234567",
        "rustc 1.86.0",
        vec![],
        vec![],
    )
    .expect("manifest")
}

fn tag() -> TagValidation {
    TagValidation {
        annotated: true,
        commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
    }
}

fn passing_gates() -> Vec<GateRun> {
    vec![
        GateRun {
            name: "fmt".to_owned(),
            passed: true,
            evidence: String::new(),
        },
        GateRun {
            name: "tests".to_owned(),
            passed: true,
            evidence: String::new(),
        },
    ]
}

#[test]
fn a_prequalified_candidate_publishes_to_the_non_public_channel() {
    let request = PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(),
        gates: passing_gates(),
        provenance_ok: true,
        tag: tag(),
        promotion: true,
    };
    let outcome = publish_prequalified(&request).expect("publish");
    assert!(outcome.published, "{outcome:?}");
    assert!(!outcome.public_channel, "non-public stays non-public");
}

#[test]
fn a_failing_gate_never_publishes() {
    let request = PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(),
        gates: vec![GateRun {
            name: "tests".to_owned(),
            passed: false,
            evidence: "gate failed".to_owned(),
        }],
        provenance_ok: true,
        tag: tag(),
        promotion: true,
    };
    let error = publish_prequalified(&request).expect_err("gate failed");
    assert!(
        matches!(error, PublishError::NotPrequalified { .. }),
        "{error}"
    );
}

#[test]
fn a_provenance_mismatch_never_publishes() {
    let request = PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(),
        gates: passing_gates(),
        provenance_ok: false,
        tag: tag(),
        promotion: true,
    };
    let error = publish_prequalified(&request).expect_err("provenance");
    assert!(
        matches!(error, PublishError::NotPrequalified { .. }),
        "{error}"
    );
}

#[test]
fn the_public_channel_requires_the_promotion_gate() {
    let request = PublishRequest {
        channel: Channel::Public,
        manifest: manifest(),
        gates: passing_gates(),
        provenance_ok: true,
        tag: tag(),
        promotion: false,
    };
    let error = publish_prequalified(&request).expect_err("no promotion");
    assert!(matches!(error, PublishError::PromotionRequired), "{error}");
}

#[test]
fn publication_never_touches_the_main_branch() {
    // The unsafe main-push tagging quarantine: publication records the
    // candidate only; it never pushes to main.
    let request = PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(),
        gates: passing_gates(),
        provenance_ok: true,
        tag: tag(),
        promotion: true,
    };
    let outcome = publish_prequalified(&request).expect("publish");
    assert!(!outcome.main_branch_touched, "no main push publication");
}
