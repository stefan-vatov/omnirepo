//! Focused proof for the generated lifecycle model, actions, and
//! executable invariants.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::lifecycle_model::{
    Action, InvariantViolation, LifecycleModel, ModelError, RepoState, RunState, assert_invariants,
    new_model, transition,
};

#[test]
fn the_canonical_happy_path_reaches_complete_without_violations() {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    let actions = vec![
        Action::StartSync {
            repository: "repo-a",
        },
        Action::RunCheck {
            repository: "repo-a",
        },
        Action::Commit {
            repository: "repo-a",
        },
        Action::Push {
            repository: "repo-a",
        },
        Action::Complete,
    ];
    for action in actions {
        transition(&mut model, action).expect("transition");
    }
    assert_eq!(model.run, RunState::Complete);
    assert!(
        assert_invariants(&model).is_empty(),
        "{:?}",
        assert_invariants(&model)
    );
}

#[test]
fn evidence_without_a_prior_intent_is_an_invariant_violation() {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    let error = transition(
        &mut model,
        Action::RecordEvidence {
            repository: "repo-a",
        },
    )
    .expect_err("evidence without intent");
    assert!(
        matches!(error, ModelError::EffectWithoutIntent { .. }),
        "{error}"
    );
    assert!(assert_invariants(&model).is_empty());
}

#[test]
fn a_cancelled_run_is_terminal_and_cannot_restart_into_sync() {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    transition(&mut model, Action::Cancel).expect("cancel");
    assert_eq!(model.run, RunState::Cancelled);
    let error = transition(
        &mut model,
        Action::StartSync {
            repository: "repo-a",
        },
    )
    .expect_err("terminal");
    assert!(matches!(error, ModelError::TerminalState { .. }), "{error}");
    assert!(assert_invariants(&model).is_empty());
}

#[test]
fn a_repository_cannot_be_in_two_states_and_unknown_repositories_fail_typed() {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    let error = transition(
        &mut model,
        Action::StartSync {
            repository: "ghost",
        },
    )
    .expect_err("unknown repo");
    assert!(
        matches!(error, ModelError::UnknownRepository { .. }),
        "{error}"
    );
    assert!(assert_invariants(&model).is_empty());
}

#[test]
fn a_crash_mid_commit_resumes_from_the_journaled_intent_on_restart() {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    transition(
        &mut model,
        Action::StartSync {
            repository: "repo-a",
        },
    )
    .expect("sync");
    transition(
        &mut model,
        Action::RunCheck {
            repository: "repo-a",
        },
    )
    .expect("check");
    transition(
        &mut model,
        Action::Commit {
            repository: "repo-a",
        },
    )
    .expect("commit intent");
    // Crash: the commit was intended but the effect never recorded.
    transition(&mut model, Action::Crash).expect("crash");
    assert_eq!(model.run, RunState::Crashed);
    assert!(assert_invariants(&model).is_empty());
    // Restart: the journaled intent is the resume point.
    transition(
        &mut model,
        Action::Restart {
            repository: "repo-a",
        },
    )
    .expect("restart");
    assert_eq!(model.run, RunState::Sync);
    let violations = assert_invariants(&model);
    assert!(violations.is_empty(), "{violations:?}");
    // The resumed run can finish the commit path.
    transition(
        &mut model,
        Action::RunCheck {
            repository: "repo-a",
        },
    )
    .expect("check");
    transition(
        &mut model,
        Action::Commit {
            repository: "repo-a",
        },
    )
    .expect("commit");
    transition(
        &mut model,
        Action::Push {
            repository: "repo-a",
        },
    )
    .expect("push");
    transition(&mut model, Action::Complete).expect("complete");
    assert!(assert_invariants(&model).is_empty());
}

#[test]
fn a_repository_cannot_be_reserved_twice() {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    transition(
        &mut model,
        Action::StartSync {
            repository: "repo-a",
        },
    )
    .expect("sync");
    transition(
        &mut model,
        Action::RunCheck {
            repository: "repo-a",
        },
    )
    .expect("check");
    transition(
        &mut model,
        Action::Commit {
            repository: "repo-a",
        },
    )
    .expect("commit");
    transition(
        &mut model,
        Action::Push {
            repository: "repo-a",
        },
    )
    .expect("push");
    transition(
        &mut model,
        Action::Repair {
            repository: "repo-a",
        },
    )
    .expect("repair");
    transition(
        &mut model,
        Action::Repair {
            repository: "repo-a",
        },
    )
    .expect_err("duplicate");
}
