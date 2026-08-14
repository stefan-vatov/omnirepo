//! Focused proof for the synthetic local fleet and source generators.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_generators::{
    ContentKind, FleetGeneration, GeneratorError, generate_machine_config,
    generate_managed_content, generate_source_catalog, materialize_fleet, remove_fleet,
};
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn the_machine_config_preserves_declared_order_and_exact_content() {
    let repositories = vec![
        ("repo-zeta".to_owned(), 3_usize),
        ("repo-alpha".to_owned(), 1),
        ("repo-mid".to_owned(), 2),
    ];
    let config = generate_machine_config(7, &repositories);
    let zeta = config.find("repo-zeta").expect("zeta present");
    let alpha = config.find("repo-alpha").expect("alpha present");
    let mid = config.find("repo-mid").expect("mid present");
    assert!(
        zeta < alpha && alpha < mid,
        "declared order is preserved: {config}"
    );
    // Exact content: regenerating with the same seed yields identical
    // bytes.
    assert_eq!(config, generate_machine_config(7, &repositories));
    assert_ne!(config, generate_machine_config(8, &repositories));
}

#[test]
fn the_source_catalog_preserves_order_and_exact_content() {
    let items = vec![
        ("managed-a.txt".to_owned(), "source-1".to_owned()),
        ("managed-b.txt".to_owned(), "source-2".to_owned()),
    ];
    let catalog = generate_source_catalog(9, &items);
    let a = catalog.find("managed-a.txt").expect("a");
    let b = catalog.find("managed-b.txt").expect("b");
    assert!(a < b, "source order is preserved: {catalog}");
    assert_eq!(catalog, generate_source_catalog(9, &items));
}

#[test]
fn managed_content_is_deterministic_at_the_chosen_size() {
    let small = generate_managed_content(5, 16, ContentKind::WholeFile);
    let large = generate_managed_content(5, 64 * 1024, ContentKind::WholeFile);
    assert_eq!(small.len(), 16, "small content is exactly 16 bytes");
    assert_eq!(large.len(), 64 * 1024, "large content is exactly 64 KiB");
    assert_eq!(
        small,
        generate_managed_content(5, 16, ContentKind::WholeFile)
    );
    assert_ne!(
        small,
        generate_managed_content(6, 16, ContentKind::WholeFile)
    );
    let section = generate_managed_content(5, 16, ContentKind::Section);
    assert_ne!(small, section, "the section kind differs from whole-file");
}

#[test]
fn generated_fleet_identities_are_unique_and_alias_cases_are_explicit() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("fleet-gen-")
        .tempdir_in(&base)
        .expect("fixture");
    let fleet = materialize_fleet(13, 200, fixture.path()).expect("fleet");
    let mut identities = BTreeSet::new();
    for entry in fs::read_dir(&fleet.root).expect("read fleet") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            assert!(
                identities.insert(name.clone()),
                "duplicate fleet identity {name}"
            );
            assert!(
                !name.contains("..") && !name.contains('/'),
                "no alias/traversal identity: {name}"
            );
        }
    }
    assert_eq!(identities.len(), 200, "every repository identity is unique");
    remove_fleet(&fleet).expect("cleanup");
    assert!(!fleet.root.exists(), "cleanup removes the generated fleet");
}

#[test]
fn seeds_reproduce_the_same_generated_fleet() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("fleet-gen-repro-")
        .tempdir_in(&base)
        .expect("fixture");
    let first = materialize_fleet(21, 3, fixture.path()).expect("first");
    let read = |fleet: &FleetGeneration| -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        let mut paths = fs::read_dir(&fleet.root)
            .expect("read")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                let mut children = fs::read_dir(&path)
                    .expect("read")
                    .map(|entry| entry.expect("entry").path())
                    .collect::<Vec<_>>();
                children.sort();
                for child in children {
                    files.push((
                        child
                            .file_name()
                            .expect("name")
                            .to_string_lossy()
                            .into_owned(),
                        fs::read(&child).expect("content"),
                    ));
                }
            }
        }
        files
    };
    let first_read = read(&first);
    remove_fleet(&first).expect("cleanup");
    let second = materialize_fleet(21, 3, fixture.path()).expect("second");
    assert_eq!(
        first_read,
        read(&second),
        "the same seed reproduces the fleet"
    );
    remove_fleet(&second).expect("cleanup");
}
