//! Focused proof for modeling setup intent and existing-state effect
//! plans.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::setup_plan::{
    ExistingFile, SetupAction, SetupIntent, SetupPlanError, compute_setup_plan,
};

#[test]
fn absent_canonical_files_plan_create_actions() {
    let intent = SetupIntent::machine("machine-a", "version: 1\nrepositories: []\n");
    let plan = compute_setup_plan(&intent, &[]).expect("plan");
    assert_eq!(plan.len(), 1, "exactly the selected file: {plan:?}");
    assert!(
        matches!(&plan[0], SetupAction::Create { path, .. } if path == "machine-a"),
        "{plan:?}"
    );
}

#[test]
fn an_identical_existing_file_plans_a_no_op() {
    let intent = SetupIntent::machine("machine-a", "version: 1\nrepositories: []\n");
    let existing = vec![ExistingFile {
        path: "machine-a".to_owned(),
        content: "version: 1\nrepositories: []\n".to_owned(),
        valid: true,
    }];
    let plan = compute_setup_plan(&intent, &existing).expect("plan");
    assert!(
        matches!(&plan[0], SetupAction::NoOp { path, .. } if path == "machine-a"),
        "repeated setup is a no-op: {plan:?}"
    );
}

#[test]
fn a_valid_but_different_existing_file_plans_an_update() {
    let intent = SetupIntent::machine("machine-a", "version: 1\nrepositories: []\n");
    let existing = vec![ExistingFile {
        path: "machine-a".to_owned(),
        content: "version: 1\nrepositories:\n  - id: old\n    path: /srv/old\n".to_owned(),
        valid: true,
    }];
    let plan = compute_setup_plan(&intent, &existing).expect("plan");
    assert!(
        matches!(&plan[0], SetupAction::Update { path, .. } if path == "machine-a"),
        "{plan:?}"
    );
}

#[test]
fn an_invalid_or_conflicting_existing_file_is_refused_never_replaced() {
    let intent = SetupIntent::machine("machine-a", "version: 1\nrepositories: []\n");
    let existing = vec![ExistingFile {
        path: "machine-a".to_owned(),
        content: "not: [valid\n".to_owned(),
        valid: false,
    }];
    let error = compute_setup_plan(&intent, &existing).expect_err("refused");
    assert!(
        matches!(
            &error,
            SetupPlanError::ConflictingAuthority { path }
                if path == "machine-a"
        ),
        "{error}"
    );
}

#[test]
fn only_explicitly_selected_files_appear_in_the_plan() {
    let intent = SetupIntent::machine("machine-a", "version: 1\n");
    // An ambient unrelated file in the home must never enter the plan.
    let existing = vec![ExistingFile {
        path: "unrelated.txt".to_owned(),
        content: "x".to_owned(),
        valid: true,
    }];
    let plan = compute_setup_plan(&intent, &existing).expect("plan");
    assert_eq!(plan.len(), 1);
    assert!(
        matches!(&plan[0], SetupAction::Create { path, .. } if path == "machine-a"),
        "no ambient discovery: {plan:?}"
    );
}
