//! Focused proof for binding source declarations to configured
//! repositories by applicability.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, DestinationRepository, MachineConcurrency, MachineConfiguration, RepairControls,
    RepositoryId, RepositoryTag, SchemaVersion,
};
use crate::lifecycle::fleet_binding::bind_declarations;
use crate::source::{ItemDeclaration, ItemKind, RevisionId, SourceDeclaration, SourceId};
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-binding-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("source id")
}

fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("revision")
}

fn tag(value: &str) -> RepositoryTag {
    RepositoryTag::parse(value).expect("tag")
}

fn destination(root: &Path, id: &str, tags: &[&str]) -> DestinationRepository {
    let path = root.join(id);
    fs::create_dir_all(&path).expect("destination");
    DestinationRepository::new(
        RepositoryId::parse(id).expect("repository id"),
        AbsolutePath::parse(path.to_str().expect("utf8")).expect("path"),
        tags.iter().map(|value| tag(value)).collect::<Vec<_>>(),
    )
    .expect("destination")
}

fn machine(repositories: Vec<DestinationRepository>) -> MachineConfiguration {
    MachineConfiguration::new(
        SchemaVersion::new(1).expect("version"),
        repositories,
        Vec::new(),
        None,
        MachineConcurrency::new(4, 8).expect("concurrency"),
        RepairControls::default(),
    )
    .expect("machine")
}

fn declaration(
    source: &str,
    index: usize,
    id: &str,
    destination: &str,
    mode: &str,
    tags: &str,
) -> SourceDeclaration {
    SourceDeclaration {
        source: source_id(source),
        revision: revision("rev-1"),
        path: format!("src/{id}.yaml"),
        fields: vec![
            ("id".to_owned(), id.to_owned()),
            ("mode".to_owned(), mode.to_owned()),
            ("destination".to_owned(), destination.to_owned()),
            ("tags".to_owned(), tags.to_owned()),
        ],
        provenance: format!("source.yaml:{}", index + 1),
    }
}

#[test]
fn declarations_bind_to_matching_destinations_via_stable_tags() {
    let fixture = fixture_base();
    let config = machine(vec![
        destination(fixture.path(), "repo-a", &["frontend"]),
        destination(fixture.path(), "repo-b", &["backend"]),
    ]);
    let declarations = vec![
        declaration("source-a", 0, "item-1", "apps/app.yaml", "sync", "frontend"),
        declaration("source-a", 1, "item-2", "apps/api.yaml", "sync", "backend"),
    ];
    let bindings = bind_declarations(&config, &declarations).expect("bindings");
    // repo-a receives item-1 only; repo-b receives item-2 only.
    let repo_a = bindings
        .iter()
        .find(|(repository, _)| repository == "repo-a")
        .expect("repo-a");
    assert_eq!(repo_a.1.len(), 1);
    assert_eq!(repo_a.1[0].id, "item-1");
    assert_eq!(repo_a.1[0].target, "apps/app.yaml");
    assert_eq!(repo_a.1[0].source, "source-a");
    let repo_b = bindings
        .iter()
        .find(|(repository, _)| repository == "repo-b")
        .expect("repo-b");
    assert_eq!(repo_b.1.len(), 1);
    assert_eq!(repo_b.1[0].id, "item-2");
}

#[test]
fn untagged_declarations_apply_to_every_destination() {
    let fixture = fixture_base();
    let config = machine(vec![
        destination(fixture.path(), "repo-a", &["frontend"]),
        destination(fixture.path(), "repo-b", &["backend"]),
    ]);
    let declarations = vec![declaration(
        "source-a",
        0,
        "item-common",
        "README.md",
        "sync",
        "",
    )];
    let bindings = bind_declarations(&config, &declarations).expect("bindings");
    for (_, items) in &bindings {
        assert_eq!(
            items.len(),
            1,
            "the common item applies to every destination"
        );
        assert_eq!(items[0].id, "item-common");
    }
}

#[test]
fn the_item_declaration_conversion_is_exact() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a", &[])]);
    let mut section_declaration =
        declaration("source-a", 0, "item-sec", "apps/app.yaml", "section", "");
    section_declaration
        .fields
        .push(("section".to_owned(), "rules".to_owned()));
    let declarations = vec![
        section_declaration,
        declaration("source-a", 1, "item-whole", "apps/other.yaml", "sync", ""),
    ];
    let bindings = bind_declarations(&config, &declarations).expect("bindings");
    assert_eq!(bindings[0].1.len(), 2);
    assert_eq!(bindings[0].1[0].kind, ItemKind::Section);
    assert_eq!(
        bindings[0].1[0]
            .section
            .as_ref()
            .map(|section| section.as_str()),
        Some("rules"),
        "the section id is carried from the declaration"
    );
    assert_eq!(bindings[0].1[0].source_order, 0);
    assert_eq!(bindings[0].1[1].kind, ItemKind::WholeFile);
    assert_eq!(bindings[0].1[1].section, None);
    assert_eq!(
        bindings[0].1[1].source_order, 1,
        "declared order is preserved"
    );
}

#[test]
fn section_declarations_require_a_valid_section_id() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a", &[])]);
    // mode=section without a section id fails typed.
    let missing = vec![declaration(
        "source-a",
        0,
        "item-sec",
        "apps/app.yaml",
        "section",
        "",
    )];
    assert!(matches!(
        bind_declarations(&config, &missing),
        Err(super::BindingError::MissingSectionId { .. })
    ));
    // An invalid section id fails typed.
    let mut invalid = declaration("source-a", 0, "item-sec", "apps/app.yaml", "section", "");
    invalid
        .fields
        .push(("section".to_owned(), "Not Valid".to_owned()));
    assert!(matches!(
        bind_declarations(&config, &[invalid]),
        Err(super::BindingError::InvalidSectionId { .. })
    ));
    // A section id on a whole-file declaration fails typed.
    let mut misplaced = declaration("source-a", 0, "item-whole", "apps/app.yaml", "sync", "");
    misplaced
        .fields
        .push(("section".to_owned(), "rules".to_owned()));
    assert!(matches!(
        bind_declarations(&config, &[misplaced]),
        Err(super::BindingError::SectionOnWholeFile { .. })
    ));
}

#[test]
fn a_malformed_declaration_fails_typed_for_its_source() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a", &[])]);
    let declarations = vec![SourceDeclaration {
        source: source_id("source-a"),
        revision: revision("rev-1"),
        path: "src/x.yaml".to_owned(),
        // No id field: the binding cannot name the item.
        fields: vec![("mode".to_owned(), "sync".to_owned())],
        provenance: "source.yaml:1".to_owned(),
    }];
    let error = bind_declarations(&config, &declarations).expect_err("malformed");
    assert!(error.to_string().contains("source-a"), "{error}");
}

#[test]
fn the_binding_never_probes_destination_content() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a", &[])]);
    // The destination exists but has no files; the binding is pure over
    // the config and the declarations and never inspects the directory.
    let declarations = vec![declaration(
        "source-a",
        0,
        "item-1",
        "apps/app.yaml",
        "sync",
        "",
    )];
    let bindings = bind_declarations(&config, &declarations).expect("bindings");
    assert_eq!(bindings[0].1.len(), 1);
    // Nothing was written or read from the destination.
    assert!(
        fs::read_dir(fixture.path().join("repo-a"))
            .expect("read")
            .next()
            .is_none()
    );
}

#[test]
fn invalid_ids_and_protected_targets_fail_typed() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a", &[])]);
    // Item IDs follow the stable slug rule.
    let upper = vec![declaration(
        "source-a",
        0,
        "Item-One",
        "apps/app.yaml",
        "sync",
        "",
    )];
    assert!(matches!(
        bind_declarations(&config, &upper),
        Err(super::BindingError::InvalidId { .. })
    ));
    // A destination's own configuration authority is protected.
    for protected in [".omnirepo.yaml", ".omnirepo/source.yaml", ".omnirepo"] {
        let declared = vec![declaration("source-a", 0, "item-1", protected, "sync", "")];
        assert!(
            matches!(
                bind_declarations(&config, &declared),
                Err(super::BindingError::ProtectedTarget { .. })
            ),
            "{protected}"
        );
    }
}
