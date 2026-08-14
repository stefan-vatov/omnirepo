//! Focused proof for seeded action generation, fault injection, and
//! counterexample shrinking over the lifecycle model.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::lifecycle_model::{
    Action, LifecycleModel, ModelError, new_model, transition,
};
use crate::lifecycle::model_generation::{
    FaultKind, generate_actions, inject_fault, shrink_counterexample,
};

fn model_runs(actions: &[Action<'_>]) -> Result<LifecycleModel, ModelError> {
    let mut model = new_model("run-1", &["repo-a".to_owned()]);
    for action in actions {
        transition(&mut model, action.clone())?;
    }
    Ok(model)
}

fn run_errors(actions: &[Action<'_>]) -> bool {
    model_runs(actions).is_err()
}

#[test]
fn seeded_generation_is_deterministic_and_closed() {
    let first = generate_actions(42, 20, &["repo-a".to_owned()]);
    let second = generate_actions(42, 20, &["repo-a".to_owned()]);
    assert_eq!(first, second, "the same seed yields the same sequence");
    let other = generate_actions(7, 20, &["repo-a".to_owned()]);
    assert_ne!(first, other, "a different seed yields a different sequence");
    // Every generated action is a declared model action (the closed set).
    for action in &first {
        assert!(
            matches!(
                action,
                Action::StartSync { .. }
                    | Action::RunCheck { .. }
                    | Action::Commit { .. }
                    | Action::Push { .. }
                    | Action::Repair { .. }
                    | Action::RecordEvidence { .. }
                    | Action::Cancel
                    | Action::Crash
                    | Action::Restart { .. }
                    | Action::Complete
            ),
            "{action:?}"
        );
    }
}

#[test]
fn fault_injection_inserts_crash_and_cancel_at_seeded_positions() {
    let actions = generate_actions(42, 10, &["repo-a".to_owned()]);
    let injected = inject_fault(&actions, 99, FaultKind::Crash);
    assert_eq!(injected.len(), actions.len() + 1, "{injected:?}");
    assert!(
        injected
            .iter()
            .any(|action| matches!(action, Action::Crash))
    );
    let cancelled = inject_fault(&actions, 99, FaultKind::Cancel);
    assert!(
        cancelled
            .iter()
            .any(|action| matches!(action, Action::Cancel))
    );
    // The fault injection is deterministic for a fixed seed.
    assert_eq!(inject_fault(&actions, 99, FaultKind::Crash), injected);
}

#[test]
fn shrinking_reduces_a_failing_sequence_to_a_minimal_counterexample() {
    // A failing sequence: evidence before intent is a transition error;
    // it fails regardless of the trailing noise.
    let failing = vec![
        Action::RecordEvidence {
            repository: "repo-a",
        },
        Action::StartSync {
            repository: "repo-a",
        },
        Action::RunCheck {
            repository: "repo-a",
        },
    ];
    assert!(run_errors(&failing));
    let minimal = shrink_counterexample(&failing, run_errors);
    assert!(run_errors(&minimal), "the shrunk sequence still fails");
    assert!(
        minimal.len() <= failing.len(),
        "shrinking never grows the sequence"
    );
    assert!(
        minimal.len() == 1,
        "the minimal counterexample is the single evidence-without-intent action: {minimal:?}"
    );
}

#[test]
fn shrinking_stops_at_the_first_irreducible_failure() {
    let failing = vec![
        Action::StartSync {
            repository: "repo-a",
        },
        Action::RecordEvidence {
            repository: "repo-a",
        },
        Action::RecordEvidence {
            repository: "repo-a",
        },
    ];
    let minimal = shrink_counterexample(&failing, run_errors);
    assert!(run_errors(&minimal));
    // No single removal keeps the failure: it is irreducible.
    for index in 0..minimal.len() {
        let mut candidate = minimal.clone();
        candidate.remove(index);
        assert!(
            !run_errors(&candidate),
            "removing index {index} should not keep the failure"
        );
    }
}
