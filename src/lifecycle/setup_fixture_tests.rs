//! Onboarding state-matrix, noninteractive, and failure tests for the
//! setup path.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::setup_author::apply_setup_plan;
use crate::lifecycle::setup_files::{author_canonical_file, is_valid_declarations, is_valid_yaml};
use crate::lifecycle::setup_plan::{SetupAction, SetupIntent, SetupPlanError};
use crate::lifecycle::setup_run::{SetupRequest, SetupRunError, run_setup};
use std::{fs, path::Path};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("setup-fixtures-")
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn the_onboarding_state_matrix_holds_for_every_canonical_file_kind() {
    let fixture = fixture_base();
    let root = fixture.path().join("repo");
    fs::create_dir_all(root.join(".omnirepo")).expect("dir");
    // Machine config: absent -> create, identical -> no-op,
    // different-valid -> update, invalid -> refuse.
    let content = "version: 1\nrepositories: []\n";
    let machine = ".omnirepo/config.yaml";
    let create = apply_setup_plan(
        &root,
        &SetupIntent::machine(machine, content),
        &crate::lifecycle::setup_author::observe_existing(&root, machine),
    )
    .expect("create");
    assert!(matches!(create, SetupAction::Create { .. }));
    let no_op = apply_setup_plan(
        &root,
        &SetupIntent::machine(machine, content),
        &crate::lifecycle::setup_author::observe_existing(&root, machine),
    )
    .expect("no-op");
    assert!(matches!(no_op, SetupAction::NoOp { .. }));
    let update = apply_setup_plan(
        &root,
        &SetupIntent::machine(
            machine,
            "version: 1\nrepositories:\n  - id: a\n    path: /srv/a\n",
        ),
        &crate::lifecycle::setup_author::observe_existing(&root, machine),
    )
    .expect("update");
    assert!(matches!(update, SetupAction::Update { .. }));
    fs::write(root.join(machine), "bogus: [x\n").expect("invalid");
    let refused = apply_setup_plan(
        &root,
        &SetupIntent::machine(machine, content),
        &crate::lifecycle::setup_author::observe_existing(&root, machine),
    );
    assert!(matches!(
        refused,
        Err(SetupPlanError::ConflictingAuthority { .. })
    ));

    // Source declarations and destination policy: the same matrix via
    // the generic author.
    let declarations = "omnirepo-declarations-v1\nsource=source-a path=managed.txt id=item-1 mode=sync destination=managed.txt\n";
    let source_action = author_canonical_file(
        &root,
        ".omnirepo/source.yaml",
        declarations,
        is_valid_declarations,
    )
    .expect("source create");
    assert!(matches!(source_action, SetupAction::Create { .. }));
    let policy_action = author_canonical_file(
        &root,
        ".omnirepo.yaml",
        "version: 1\nall: true\n",
        is_valid_yaml,
    )
    .expect("policy create");
    assert!(matches!(policy_action, SetupAction::Create { .. }));
}

#[test]
fn noninteractive_apply_requires_yes_and_is_prompt_free() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    // Without --yes the apply is refused, prompt-free.
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        true,
        false,
    );
    assert!(matches!(
        run_setup(&home, &request, None),
        Err(SetupRunError::ConfirmationRequired)
    ));
    // With --yes the apply succeeds.
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        true,
        true,
    );
    let outcome = run_setup(&home, &request, None).expect("apply");
    assert_eq!(outcome.applied.len(), 1);
    assert!(home.join(".omnirepo/config.yaml").exists());
}

#[test]
fn an_io_failure_is_typed_not_panicked() {
    let fixture = fixture_base();
    let root = fixture.path().join("blocked");
    fs::create_dir_all(&root).expect("dir");
    // A path whose parent is a regular file: the write cannot happen.
    let blocker = root.join("file");
    fs::write(&blocker, "x").expect("blocker");
    let action = author_canonical_file(&root, "file/child.yaml", "version: 1\n", is_valid_yaml);
    assert!(
        matches!(action, Err(SetupPlanError::Io { .. })),
        "{action:?}"
    );
}

#[test]
fn the_plan_only_mode_never_touches_the_filesystem() {
    let fixture = fixture_base();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let request = SetupRequest::machine(
        ".omnirepo/config.yaml".to_owned(),
        "version: 1\nrepositories: []\n".to_owned(),
        false,
        false,
    );
    let outcome = run_setup(&home, &request, None).expect("plan");
    assert!(
        outcome
            .plan
            .iter()
            .any(|action| matches!(action, SetupAction::Create { .. })),
        "{:?}",
        outcome.plan
    );
    assert!(outcome.applied.is_empty());
    assert!(!home.join(".omnirepo").exists(), "plan-only never writes");
}
