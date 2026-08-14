//! Trigger, provenance, substitution, idempotence, and channel tests
//! for the release pipeline.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::release_gates::{GateRun, verify_candidate_provenance};
use crate::lifecycle::release_manifest::{CandidateManifest, manifest_for};
use crate::lifecycle::release_publish::{
    Channel, PublishError, PublishRequest, publish_prequalified,
};
use crate::lifecycle::release_tag::{TagOutcome, create_canonical_tag, validate_canonical_tag};
use crate::lifecycle::release_trigger::verify_exact_sha_trigger;
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("release-fixtures-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(root: &Path) -> std::path::PathBuf {
    fs::create_dir_all(root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Release"]);
    git(&["config", "user.email", "release@example.test"]);
    fs::write(root.join("file.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    root.to_path_buf()
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {:?}", output);
    String::from_utf8(output.stdout)
        .expect("stdout")
        .trim()
        .to_owned()
}

fn manifest(commit: &str) -> CandidateManifest {
    manifest_for("0.9.0", commit, "rustc 1.86.0", vec![], vec![]).expect("manifest")
}

fn passing_gates() -> Vec<GateRun> {
    vec![GateRun {
        name: "tests".to_owned(),
        passed: true,
        evidence: String::new(),
    }]
}

#[test]
fn the_trigger_and_tag_and_provenance_and_publish_chain_holds() {
    let fixture = fixture_base();
    let root = git_repo(&fixture.path().join("repo"));
    let commit = git_text(&root, &["rev-parse", "HEAD"]);
    // Tag: canonical annotated tag at the exact commit; idempotent.
    let created = create_canonical_tag(root.to_str().unwrap(), "0.9.0", &commit).expect("create");
    assert!(matches!(created, TagOutcome::Created { .. }));
    let again = create_canonical_tag(root.to_str().unwrap(), "0.9.0", &commit).expect("again");
    assert!(matches!(again, TagOutcome::Existing { .. }));
    let tag = validate_canonical_tag(root.to_str().unwrap(), "0.9.0").expect("validate");
    assert!(tag.annotated && tag.commit == commit);
    // Trigger: the tag's commit and the exact-SHA input match the head.
    let trigger = verify_exact_sha_trigger(&root, "v0.9.0", &commit, &commit).expect("trigger");
    assert!(trigger.verified);
    // Provenance: the manifest matches the checkout.
    let checkout = fixture.path().join("checkout");
    fs::create_dir_all(&checkout).expect("checkout");
    fs::write(checkout.join("HEAD"), &commit).expect("head");
    verify_candidate_provenance(&manifest(&commit), &checkout).expect("provenance");
    // Publish: prequalified to the non-public channel.
    let outcome = publish_prequalified(&PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(&commit),
        gates: passing_gates(),
        provenance_ok: true,
        tag: tag.clone(),
        promotion: true,
    })
    .expect("publish");
    assert!(outcome.published && !outcome.public_channel);
    // Idempotence: publishing the same candidate again is accepted.
    let second = publish_prequalified(&PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(&commit),
        gates: passing_gates(),
        provenance_ok: true,
        tag,
        promotion: true,
    })
    .expect("publish again");
    assert!(second.published);
}

#[test]
fn a_substituted_commit_fails_every_gate() {
    let fixture = fixture_base();
    let root = git_repo(&fixture.path().join("repo"));
    let commit = git_text(&root, &["rev-parse", "HEAD"]);
    let substituted = "fedcba9876543210fedcba9876543210fedcba98";
    // Trigger refuses the substituted SHA.
    assert!(verify_exact_sha_trigger(&root, "v0.9.0", &commit, substituted).is_err());
    // Provenance refuses the substituted checkout.
    let checkout = fixture.path().join("checkout");
    fs::create_dir_all(&checkout).expect("checkout");
    fs::write(checkout.join("HEAD"), substituted).expect("head");
    assert!(verify_candidate_provenance(&manifest(&commit), &checkout).is_err());
    // Publish refuses when the tag and manifest disagree.
    let tag = crate::lifecycle::release_tag::TagValidation {
        annotated: true,
        commit: commit.clone(),
    };
    let error = publish_prequalified(&PublishRequest {
        channel: Channel::NonPublic,
        manifest: manifest(substituted),
        gates: passing_gates(),
        provenance_ok: true,
        tag,
        promotion: true,
    })
    .expect_err("substituted");
    assert!(matches!(error, PublishError::NotPrequalified { .. }));
}

#[test]
fn the_public_channel_gate_is_enforced_across_the_chain() {
    let fixture = fixture_base();
    let root = git_repo(&fixture.path().join("repo"));
    let commit = git_text(&root, &["rev-parse", "HEAD"]);
    let tag = crate::lifecycle::release_tag::TagValidation {
        annotated: true,
        commit: commit.clone(),
    };
    // Without promotion, the public channel is refused even when fully
    // prequalified otherwise.
    let error = publish_prequalified(&PublishRequest {
        channel: Channel::Public,
        manifest: manifest(&commit),
        gates: passing_gates(),
        provenance_ok: true,
        tag,
        promotion: false,
    })
    .expect_err("no promotion");
    assert!(matches!(error, PublishError::PromotionRequired));
}
