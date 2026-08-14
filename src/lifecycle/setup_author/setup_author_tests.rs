//! Focused proof for idempotent machine authority authoring.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::setup_author::{apply_setup_plan, observe_existing};
use crate::lifecycle::setup_plan::{SetupIntent, SetupPlanError};
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("setup-author-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn intent() -> SetupIntent {
    SetupIntent::machine("machine-a", "version: 1\nrepositories: []\n")
}

#[test]
fn a_missing_authority_is_created_and_repeat_apply_is_a_no_op() {
    let fixture = fixture_base();
    let path = fixture.path().join("machine-a");
    let existing = observe_existing(fixture.path(), "machine-a");
    let first = apply_setup_plan(fixture.path(), &intent(), &existing).expect("create");
    assert!(matches!(
        first,
        crate::lifecycle::setup_plan::SetupAction::Create { .. }
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("created"),
        "version: 1\nrepositories: []\n"
    );
    // Applying the same intent again is a no-op.
    let existing = observe_existing(fixture.path(), "machine-a");
    let second = apply_setup_plan(fixture.path(), &intent(), &existing).expect("no-op");
    assert!(
        matches!(
            second,
            crate::lifecycle::setup_plan::SetupAction::NoOp { .. }
        ),
        "{second:?}"
    );
}

#[test]
fn a_valid_but_different_authority_is_updated() {
    let fixture = fixture_base();
    let path = fixture.path().join("machine-a");
    fs::write(
        &path,
        "version: 1\nrepositories:\n  - id: old\n    path: /srv/old\n",
    )
    .expect("existing");
    let existing = observe_existing(fixture.path(), "machine-a");
    let action = apply_setup_plan(fixture.path(), &intent(), &existing).expect("update");
    assert!(matches!(
        action,
        crate::lifecycle::setup_plan::SetupAction::Update { .. }
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("updated"),
        "version: 1\nrepositories: []\n"
    );
}

#[test]
fn an_invalid_authority_is_never_replaced() {
    let fixture = fixture_base();
    let path = fixture.path().join("machine-a");
    fs::write(&path, "not: [valid\n").expect("invalid");
    let existing = observe_existing(fixture.path(), "machine-a");
    assert!(
        !existing[0].valid,
        "the observer detects the invalid authority"
    );
    let error = apply_setup_plan(fixture.path(), &intent(), &existing).expect_err("refused");
    assert!(
        matches!(error, SetupPlanError::ConflictingAuthority { .. }),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("untouched"),
        "not: [valid\n",
        "the invalid authority stays byte-identical"
    );
}

#[test]
fn an_identical_authority_stays_byte_identical() {
    let fixture = fixture_base();
    let path = fixture.path().join("machine-a");
    fs::write(&path, "version: 1\nrepositories: []\n").expect("existing");
    let existing = observe_existing(fixture.path(), "machine-a");
    let action = apply_setup_plan(fixture.path(), &intent(), &existing).expect("no-op");
    assert!(matches!(
        action,
        crate::lifecycle::setup_plan::SetupAction::NoOp { .. }
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged"),
        "version: 1\nrepositories: []\n"
    );
}
