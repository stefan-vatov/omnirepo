//! Focused proof for source declaration and destination policy
//! authoring.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::setup_files::{author_canonical_file, is_valid_declarations, is_valid_yaml};
use crate::lifecycle::setup_plan::{SetupAction, SetupPlanError};
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("setup-files-")
        .tempdir_in(&base)
        .expect("fixture")
}

const DECLARATIONS: &str = "omnirepo-declarations-v1\nsource=source-a path=managed.txt id=item-1 mode=sync destination=managed.txt\n";
const POLICY: &str = "version: 1\nall: true\n";

#[test]
fn source_declarations_author_create_noop_update_refuse() {
    let fixture = fixture_base();
    let root = fixture.path().join("source-a");
    fs::create_dir_all(root.join(".omnirepo")).expect("dir");
    // Create when absent.
    let action = author_canonical_file(
        &root,
        ".omnirepo/source.yaml",
        DECLARATIONS,
        is_valid_declarations,
    )
    .expect("create");
    assert!(matches!(action, SetupAction::Create { .. }));
    assert_eq!(
        fs::read_to_string(root.join(".omnirepo/source.yaml")).expect("created"),
        DECLARATIONS
    );
    // No-op when identical.
    let action = author_canonical_file(
        &root,
        ".omnirepo/source.yaml",
        DECLARATIONS,
        is_valid_declarations,
    )
    .expect("no-op");
    assert!(matches!(action, SetupAction::NoOp { .. }));
    // Update when valid but different.
    let different = "omnirepo-declarations-v1\nsource=source-a path=other.txt id=item-2 mode=sync destination=other.txt\n";
    let action = author_canonical_file(
        &root,
        ".omnirepo/source.yaml",
        different,
        is_valid_declarations,
    )
    .expect("update");
    assert!(matches!(action, SetupAction::Update { .. }));
    // Refuse an invalid authority, never replace it.
    fs::write(root.join(".omnirepo/source.yaml"), "garbage\n").expect("invalid");
    let error = author_canonical_file(
        &root,
        ".omnirepo/source.yaml",
        DECLARATIONS,
        is_valid_declarations,
    )
    .expect_err("refused");
    assert!(
        matches!(error, SetupPlanError::ConflictingAuthority { .. }),
        "{error}"
    );
}

#[test]
fn destination_policy_author_create_noop_update_refuse() {
    let fixture = fixture_base();
    let root = fixture.path().join("destination-a");
    fs::create_dir_all(&root).expect("dir");
    let action =
        author_canonical_file(&root, ".omnirepo.yaml", POLICY, is_valid_yaml).expect("create");
    assert!(matches!(action, SetupAction::Create { .. }));
    assert_eq!(
        fs::read_to_string(root.join(".omnirepo.yaml")).expect("created"),
        POLICY
    );
    let action =
        author_canonical_file(&root, ".omnirepo.yaml", POLICY, is_valid_yaml).expect("no-op");
    assert!(matches!(action, SetupAction::NoOp { .. }));
    let different = "version: 1\nallow:\n  - item-1\n";
    let action =
        author_canonical_file(&root, ".omnirepo.yaml", different, is_valid_yaml).expect("update");
    assert!(matches!(action, SetupAction::Update { .. }));
    fs::write(root.join(".omnirepo.yaml"), "bogus: [x\n").expect("invalid");
    let error =
        author_canonical_file(&root, ".omnirepo.yaml", POLICY, is_valid_yaml).expect_err("refused");
    assert!(
        matches!(error, SetupPlanError::ConflictingAuthority { .. }),
        "{error}"
    );
}

#[test]
fn validity_checks_are_specific_to_each_file_kind() {
    assert!(is_valid_declarations(DECLARATIONS));
    assert!(!is_valid_declarations("garbage\n"));
    assert!(is_valid_yaml(POLICY));
    assert!(!is_valid_yaml("bogus: [x\n"));
}
