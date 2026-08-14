//! Seeded action generation, fault injection, and counterexample
//! shrinking over the lifecycle model.
//!
//! The generated suite is deterministic: the same seed produces the same
//! action sequence over the closed model action set.  Fault injection
//! inserts crash or cancel actions at a seeded position.  When a
//! property fails, shrinking reduces the sequence greedily to a minimal
//! counterexample (no single removal still fails).

#![allow(dead_code)]

#[cfg(test)]
mod model_generation_tests;

use crate::lifecycle::lifecycle_model::{Action, LifecycleModel, transition};

/// The injected fault kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultKind {
    Crash,
    Cancel,
}

/// Deterministic split-mix style PRNG for generation.
struct SeededRandom(u64);

impl SeededRandom {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn pick(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn is_terminal(action: &Action<'_>) -> bool {
    matches!(action, Action::Cancel | Action::Crash | Action::Complete)
}

/// Generate a deterministic action sequence over the closed action set.
///
/// The repository actions target the declared repositories; terminal
/// actions appear rarely so the generated runs mostly exercise the
/// forward path.  Pure and deterministic for a fixed seed.
pub fn generate_actions(seed: u64, length: usize, repositories: &[String]) -> Vec<Action<'static>> {
    let mut random = SeededRandom(seed);
    let mut actions = Vec::new();
    let repository = repositories
        .first()
        .cloned()
        .unwrap_or_else(|| "repo".to_owned());
    let repository: &'static str = Box::leak(repository.into_boxed_str());
    for index in 0..length {
        let choice = random.pick(10);
        let action = match choice {
            0 => Action::StartSync { repository },
            1 => Action::RunCheck { repository },
            2 => Action::Commit { repository },
            3 => Action::Push { repository },
            4 => Action::Repair { repository },
            5 => Action::RecordEvidence { repository },
            6 => Action::Restart { repository },
            7 if index % 5 == 0 => Action::Crash,
            8 if index % 7 == 0 => Action::Cancel,
            _ => Action::StartSync { repository },
        };
        actions.push(action);
    }
    actions
}

/// Inject one fault into the sequence at a seeded position.
///
/// The fault (crash or cancel) is inserted; deterministic for a fixed
/// seed.  When the sequence is empty the fault is the only action.
pub fn inject_fault(actions: &[Action<'_>], seed: u64, fault: FaultKind) -> Vec<Action<'static>> {
    let mut random = SeededRandom(seed);
    let position = random.pick(actions.len() + 1);
    let injected = match fault {
        FaultKind::Crash => Action::Crash,
        FaultKind::Cancel => Action::Cancel,
    };
    let mut result = actions
        .iter()
        .map(action_to_static)
        .collect::<Vec<Action<'static>>>();
    result.insert(position, injected);
    result
}

fn action_to_static(action: &Action<'_>) -> Action<'static> {
    match action {
        Action::StartSync { repository } => Action::StartSync {
            repository: repository.clone_box(),
        },
        Action::RunCheck { repository } => Action::RunCheck {
            repository: repository.clone_box(),
        },
        Action::Commit { repository } => Action::Commit {
            repository: repository.clone_box(),
        },
        Action::Push { repository } => Action::Push {
            repository: repository.clone_box(),
        },
        Action::Repair { repository } => Action::Repair {
            repository: repository.clone_box(),
        },
        Action::RecordEvidence { repository } => Action::RecordEvidence {
            repository: repository.clone_box(),
        },
        Action::Cancel => Action::Cancel,
        Action::Crash => Action::Crash,
        Action::Restart { repository } => Action::Restart {
            repository: repository.clone_box(),
        },
        Action::Complete => Action::Complete,
    }
}

trait CloneBox {
    fn clone_box(&self) -> &'static str;
}

impl CloneBox for &str {
    fn clone_box(&self) -> &'static str {
        Box::leak((*self).to_owned().into_boxed_str())
    }
}

/// Shrink a failing sequence greedily to a minimal counterexample.
///
/// The property decides failure.  Each pass tries removing every action;
/// the first removal that keeps the failure is kept and the pass repeats.
/// The result is irreducible: no single removal still fails.  Pure.
pub fn shrink_counterexample(
    actions: &[Action<'_>],
    property: impl Fn(&[Action<'_>]) -> bool,
) -> Vec<Action<'static>> {
    let mut current = actions
        .iter()
        .map(action_to_static)
        .collect::<Vec<Action<'static>>>();
    let mut changed = true;
    while changed {
        changed = false;
        for index in 0..current.len() {
            let mut candidate = current.clone();
            candidate.remove(index);
            if !candidate.is_empty() && property(&candidate) {
                current = candidate;
                changed = true;
                break;
            }
        }
    }
    current
}

/// Run a generated sequence over the model; returns the terminal model.
#[allow(dead_code)]
pub fn run_generated(model: &mut LifecycleModel, actions: &[Action<'_>]) -> Result<(), ()> {
    for action in actions {
        transition(model, action.clone()).map_err(|_| ())?;
        if is_terminal(action) {
            break;
        }
    }
    Ok(())
}
