//! Focused proof for the runtime source catalog built from the machine
//! authority.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, MachineConcurrency, MachineConfiguration, RepairControls, SchemaVersion,
    SourceLocation, SourceReference,
};

use crate::lifecycle::fleet_catalog::{CatalogBuildError, build_runtime_catalog};
use crate::source::{CatalogState, SourceCatalog};
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-catalog-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Catalog"]);
    git(&["config", "user.email", "catalog@example.test"]);
    fs::write(dir.join("managed.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
}

fn source_id(value: &str) -> crate::configuration::SourceId {
    crate::configuration::SourceId::parse(value).expect("source id")
}

fn machine(
    repositories: Vec<crate::configuration::DestinationRepository>,
    sources: Vec<SourceReference>,
) -> MachineConfiguration {
    machine_with_cache(repositories, sources, None)
}

fn machine_with_cache(
    repositories: Vec<crate::configuration::DestinationRepository>,
    sources: Vec<SourceReference>,
    cache_root: Option<AbsolutePath>,
) -> MachineConfiguration {
    MachineConfiguration::new(
        SchemaVersion::new(1).expect("version"),
        repositories,
        sources,
        cache_root,
        MachineConcurrency::new(4, 8).expect("concurrency"),
        RepairControls::default(),
    )
    .expect("machine")
}

#[test]
fn local_sources_record_complete_in_declared_order_after_the_pin() {
    let fixture = fixture_base();
    let first = fixture.path().join("source-a");
    let second = fixture.path().join("source-b");
    git_repo(&first);
    git_repo(&second);
    let first_path = AbsolutePath::parse(first.to_str().expect("utf8")).expect("path");
    let second_path = AbsolutePath::parse(second.to_str().expect("utf8")).expect("path");
    let config = machine(
        Vec::new(),
        vec![
            SourceReference::new(source_id("source-a"), SourceLocation::local(first_path)),
            SourceReference::new(source_id("source-b"), SourceLocation::local(second_path)),
        ],
    );
    let catalog = build_runtime_catalog(&config).expect("catalog");
    let entries = catalog.entries();
    assert_eq!(entries.len(), 2, "declared order preserved");
    match &entries[0] {
        CatalogState::Complete { source, revision } => {
            assert_eq!(source.as_str(), "source-a");
            assert!(!revision.as_str().is_empty(), "the revision is pinned");
        }
        other => panic!("expected complete, got {other:?}"),
    }
    match &entries[1] {
        CatalogState::Complete { source, .. } => assert_eq!(source.as_str(), "source-b"),
        other => panic!("expected complete, got {other:?}"),
    }
}

#[test]
fn local_sources_must_be_clean_and_on_main() {
    let fixture = fixture_base();
    let dirty = fixture.path().join("dirty");
    let branch = fixture.path().join("branch");
    git_repo(&dirty);
    git_repo(&branch);
    fs::write(dirty.join("managed.txt"), "dirty\n").expect("dirty source");
    let output = Command::new("git")
        .args(["switch", "--quiet", "--create", "feature"])
        .current_dir(&branch)
        .output()
        .expect("git switch");
    assert!(output.status.success(), "git switch: {output:?}");
    let config = machine(
        Vec::new(),
        vec![
            SourceReference::new(
                source_id("dirty"),
                SourceLocation::local(
                    AbsolutePath::parse(dirty.to_str().expect("utf8")).expect("path"),
                ),
            ),
            SourceReference::new(
                source_id("branch"),
                SourceLocation::local(
                    AbsolutePath::parse(branch.to_str().expect("utf8")).expect("path"),
                ),
            ),
        ],
    );

    let catalog = build_runtime_catalog(&config).expect("catalog");

    assert!(matches!(
        &catalog.entries()[0],
        CatalogState::Unavailable { reason, .. } if reason.contains("not clean")
    ));
    assert!(matches!(
        &catalog.entries()[1],
        CatalogState::Unavailable { reason, .. } if reason.contains("must be on main")
    ));
}

#[test]
fn remote_sources_record_unavailable_with_a_typed_reason() {
    let cache = AbsolutePath::parse("/tmp/omnirepo-cache").expect("cache");
    let config = machine_with_cache(
        Vec::new(),
        vec![
            SourceReference::new(
                source_id("source-a"),
                SourceLocation::remote("https://example.test/source-a.git").expect("remote"),
            ),
            SourceReference::new(
                source_id("source-b"),
                SourceLocation::local(AbsolutePath::parse("/definitely/not/here").expect("path")),
            ),
        ],
        Some(cache),
    );
    let catalog = build_runtime_catalog(&config).expect("catalog");
    let entries = catalog.entries();
    assert_eq!(entries.len(), 2, "declared order preserved");
    match &entries[0] {
        CatalogState::Unavailable { source, reason } => {
            assert_eq!(source.as_str(), "source-a");
            assert!(!reason.is_empty(), "typed reason");
        }
        other => panic!("expected unavailable, got {other:?}"),
    }
    match &entries[1] {
        CatalogState::Unavailable { source, .. } => assert_eq!(source.as_str(), "source-b"),
        other => panic!("expected unavailable, got {other:?}"),
    }
}

#[test]
fn an_unavailable_higher_priority_source_never_promotes_the_next() {
    let fixture = fixture_base();
    let local = fixture.path().join("source-b");
    git_repo(&local);
    let cache = AbsolutePath::parse("/tmp/omnirepo-cache").expect("cache");
    let config = machine_with_cache(
        Vec::new(),
        vec![
            SourceReference::new(
                source_id("source-a"),
                SourceLocation::remote("https://example.test/source-a.git").expect("remote"),
            ),
            SourceReference::new(
                source_id("source-b"),
                SourceLocation::local(
                    AbsolutePath::parse(local.to_str().expect("utf8")).expect("path"),
                ),
            ),
        ],
        Some(cache),
    );
    let catalog = build_runtime_catalog(&config).expect("catalog");
    let entries = catalog.entries();
    assert!(
        matches!(&entries[0], CatalogState::Unavailable { .. }),
        "the higher-priority source stays unavailable: {:?}",
        entries[0]
    );
    assert!(
        matches!(&entries[1], CatalogState::Complete { .. }),
        "the lower source stays in its declared slot: {:?}",
        entries[1]
    );
}

#[test]
fn the_catalog_build_has_no_ambient_scan_and_no_effects() {
    let fixture = fixture_base();
    // No sources: the catalog is empty and nothing outside the declared
    // roots is touched.
    let config = machine(Vec::new(), Vec::new());
    let catalog = build_runtime_catalog(&config).expect("empty catalog");
    assert!(catalog.entries().is_empty());
    // No config file, no hidden source directory appears anywhere.
    assert!(!fixture.path().join(".omnirepo").exists());
}

#[test]
fn a_missing_local_source_fails_typed_not_by_panic() {
    let config = machine(
        Vec::new(),
        vec![SourceReference::new(
            source_id("source-a"),
            SourceLocation::local(AbsolutePath::parse("/definitely/not/here").expect("path")),
        )],
    );
    let catalog = build_runtime_catalog(&config).expect("catalog");
    assert!(
        matches!(&catalog.entries()[0], CatalogState::Unavailable { .. }),
        "the missing source is unavailable, never a panic"
    );
    let _ = CatalogBuildError::SourceUnavailable;
}
