//! Focused proof for the hostile authority and filesystem fixture
//! corpus.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::hostile_fixtures::{
    Capability, FixtureKind, HostileFixture, hostile_corpus, materialize,
};
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn every_corpus_entry_documents_its_attack_and_expected_fail_boundary() {
    let corpus = hostile_corpus();
    assert!(!corpus.is_empty());
    for fixture in &corpus {
        assert!(!fixture.name.is_empty(), "every fixture has a name");
        assert!(
            !fixture.attack.is_empty(),
            "{} documents its attack",
            fixture.name
        );
        assert!(
            !fixture.expected_fail_boundary.is_empty(),
            "{} documents its expected fail boundary",
            fixture.name
        );
    }
}

#[test]
fn every_required_hostile_class_is_present() {
    let corpus = hostile_corpus();
    let kinds = corpus
        .iter()
        .map(|fixture| fixture.kind)
        .collect::<BTreeSet<_>>();
    for required in [
        FixtureKind::MachineConfig,
        FixtureKind::SourceConfig,
        FixtureKind::RepositoryConfig,
        FixtureKind::Traversal,
        FixtureKind::Symlink,
        FixtureKind::HardLink,
        FixtureKind::SpecialFile,
        FixtureKind::CaseCollision,
        FixtureKind::UnicodeCollision,
        FixtureKind::SourceDeclaration,
        FixtureKind::GitConfig,
        FixtureKind::GitAttributes,
        FixtureKind::GitHooks,
        FixtureKind::RecordPath,
    ] {
        assert!(
            kinds.contains(&required),
            "missing hostile class {required:?}"
        );
    }
}

#[test]
fn secret_sentinels_are_unique_across_the_corpus() {
    let corpus = hostile_corpus();
    let mut sentinels = BTreeSet::new();
    for fixture in &corpus {
        assert!(
            !fixture.secret_sentinel.is_empty(),
            "{} has a sentinel",
            fixture.name
        );
        assert!(
            sentinels.insert(fixture.secret_sentinel.to_owned()),
            "duplicate sentinel {:?}",
            fixture.secret_sentinel
        );
    }
}

#[test]
fn platform_specific_cases_are_capability_tagged() {
    let corpus = hostile_corpus();
    for fixture in &corpus {
        match fixture.kind {
            FixtureKind::Symlink | FixtureKind::HardLink | FixtureKind::SpecialFile => {
                assert!(
                    fixture.capability.is_some(),
                    "{} must be capability-tagged",
                    fixture.name
                );
            }
            _ => {}
        }
    }
}

#[test]
fn materialized_fixtures_cannot_escape_the_harness_root() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("hostile-corpus-")
        .tempdir_in(&base)
        .expect("fixture");
    for entry in hostile_corpus() {
        if entry.materializable() {
            materialize(&entry, fixture.path()).expect("materialize");
        }
    }
    // Every created path stays below the harness root.
    let mut pending = vec![fixture.path().to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("read") {
            let path = entry.expect("entry").path();
            assert!(
                path.starts_with(fixture.path()),
                "fixture escaped the harness root: {}",
                path.display()
            );
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
}

#[test]
fn traversal_fixtures_are_rejected_before_any_write() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("hostile-traversal-")
        .tempdir_in(&base)
        .expect("fixture");
    let traversal = hostile_corpus()
        .into_iter()
        .find(|entry| entry.kind == FixtureKind::Traversal)
        .expect("traversal fixture");
    assert!(materialize(&traversal, fixture.path()).is_err());
}
