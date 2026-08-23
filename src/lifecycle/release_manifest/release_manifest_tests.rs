//! Focused proof for the release-candidate manifest and exact identity.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_manifest::{
    ArtifactRef, CandidateManifest, GateResult, ManifestError, content_hash, exact_identity,
    manifest_for,
};

#[test]
fn the_manifest_carries_the_exact_identity_fields() {
    let manifest = manifest_for(
        "0.9.0",
        "0123456789abcdef0123456789abcdef01234567",
        "rustc 1.86.0",
        vec![ArtifactRef {
            name: "omnirepo-linux-x86_64".to_owned(),
            sha256: "ab".to_owned(),
        }],
        vec![GateResult {
            name: "quality".to_owned(),
            passed: true,
        }],
    )
    .expect("manifest");
    assert_eq!(manifest.schema, "omnirepo.release-candidate.v1");
    assert_eq!(manifest.identity.version, "0.9.0");
    assert_eq!(
        manifest.identity.source_commit,
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(manifest.identity.toolchain, "rustc 1.86.0");
    assert_eq!(manifest.artifacts.len(), 1);
    assert_eq!(manifest.gates.len(), 1);
    assert_eq!(manifest.identity.manifest_sha256.len(), 64);
    assert!(
        manifest
            .identity
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn the_exact_identity_is_deterministic() {
    let first = manifest_for(
        "0.9.0",
        "0123456789abcdef0123456789abcdef01234567",
        "rustc 1.86.0",
        vec![ArtifactRef {
            name: "omnirepo-linux-x86_64".to_owned(),
            sha256: "ab".to_owned(),
        }],
        vec![GateResult {
            name: "quality".to_owned(),
            passed: true,
        }],
    )
    .expect("manifest");
    let second = manifest_for(
        "0.9.0",
        "0123456789abcdef0123456789abcdef01234567",
        "rustc 1.86.0",
        vec![ArtifactRef {
            name: "omnirepo-linux-x86_64".to_owned(),
            sha256: "ab".to_owned(),
        }],
        vec![GateResult {
            name: "quality".to_owned(),
            passed: true,
        }],
    )
    .expect("manifest");
    assert_eq!(
        first, second,
        "identical inputs yield an identical manifest"
    );
    assert_eq!(
        exact_identity(&first),
        exact_identity(&second),
        "the exact identity is deterministic"
    );
    // A different source commit changes the identity.
    let other = manifest_for(
        "0.9.0",
        "fedcba9876543210fedcba9876543210fedcba98",
        "rustc 1.86.0",
        vec![ArtifactRef {
            name: "omnirepo-linux-x86_64".to_owned(),
            sha256: "ab".to_owned(),
        }],
        vec![GateResult {
            name: "quality".to_owned(),
            passed: true,
        }],
    )
    .expect("manifest");
    assert_ne!(exact_identity(&first), exact_identity(&other));
}

#[test]
fn the_manifest_hash_binds_schema_and_field_boundaries() {
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let first = manifest_for(
        "0.9.0",
        commit,
        "rustc 1.86.0",
        vec![ArtifactRef {
            name: "a".to_owned(),
            sha256: "bc".to_owned(),
        }],
        Vec::new(),
    )
    .expect("first manifest");
    let second = manifest_for(
        "0.9.0",
        commit,
        "rustc 1.86.0",
        vec![ArtifactRef {
            name: "ab".to_owned(),
            sha256: "c".to_owned(),
        }],
        Vec::new(),
    )
    .expect("second manifest");
    assert_ne!(
        first.identity.manifest_sha256, second.identity.manifest_sha256,
        "field boundaries must change the digest"
    );

    let mut changed_schema = first.clone();
    changed_schema.schema = "omnirepo.release-candidate.v2".to_owned();
    assert_ne!(
        first.identity.manifest_sha256,
        content_hash(&changed_schema),
        "the schema must be part of the digest"
    );
}

#[test]
fn an_invalid_version_fails_typed() {
    for version in ["", "not-a-version", "1.a.0", "01.2.3", "1.2"] {
        assert!(matches!(
            manifest_for(
                version,
                "0123456789abcdef0123456789abcdef01234567",
                "rustc 1.86.0",
                Vec::new(),
                Vec::new(),
            ),
            Err(ManifestError::InvalidVersion { .. })
        ));
    }
}

#[test]
fn a_pending_candidate_with_no_artifacts_is_valid() {
    let manifest = manifest_for(
        "0.9.0-rc.1",
        "0123456789abcdef0123456789abcdef01234567",
        "rustc 1.86.0",
        Vec::new(),
        Vec::new(),
    )
    .expect("pending");
    assert!(manifest.artifacts.is_empty());
    assert!(manifest.gates.is_empty());
    assert!(!exact_identity(&manifest).is_empty());
}
