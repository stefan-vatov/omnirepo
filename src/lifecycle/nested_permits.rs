//! Nested child-work permits and per-repository serialization.
//!
//! Child work (verifier, Git, source, agent) is bounded by a global limit
//! and per-kind limits.  Acquisition is atomic under one ledger lock with a
//! deterministic order, so permit acquisition cannot deadlock.  One
//! repository may hold at most one stage permit at a time (the mutation
//! lifecycle is serialized per repository).  Cancellation stops new
//! acquisition; a permit is released only after the caller confirms that
//! its descendants terminated.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

/// The child-work kinds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WorkKind {
    Verify,
    Git,
    Source,
    Agent,
}

impl WorkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Git => "git",
            Self::Source => "source",
            Self::Agent => "agent",
        }
    }
}

/// Nested-permit failures.
#[derive(Debug)]
pub enum PermitError {
    RunCancelled,
    DescendantsActive { repository: String },
    NotHeld { repository: String },
}

impl fmt::Display for PermitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunCancelled => write!(formatter, "no new child work: the run is cancelled"),
            Self::DescendantsActive { repository } => write!(
                formatter,
                "repository {repository} cannot release its permit: descendants still active"
            ),
            Self::NotHeld { repository } => {
                write!(
                    formatter,
                    "repository {repository} does not hold the permit"
                )
            }
        }
    }
}
impl Error for PermitError {}

/// The nested ledger state.
#[derive(Debug, Default)]
struct NestedState {
    global_active: usize,
    global_limit: usize,
    kind_active: BTreeMap<WorkKind, usize>,
    kind_limits: BTreeMap<WorkKind, usize>,
    per_repository: BTreeMap<String, WorkKind>,
    cancelled: bool,
}

/// One held child-work permit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildPermit {
    pub kind: WorkKind,
    pub repository: String,
}

/// The nested permit ledger for one run.
#[derive(Clone, Debug)]
pub struct NestedPermits {
    state: Arc<Mutex<NestedState>>,
}

impl NestedPermits {
    /// Zero limits are refused here: zero/invalid cases are handled before
    /// admission.
    pub fn new(
        global_limit: usize,
        kind_limits: &[(WorkKind, usize)],
    ) -> Result<Self, PermitError> {
        if global_limit == 0 {
            return Err(PermitError::RunCancelled);
        }
        let kind_limits: BTreeMap<WorkKind, usize> = kind_limits.iter().copied().collect();
        for limit in kind_limits.values() {
            if *limit == 0 {
                return Err(PermitError::RunCancelled);
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(NestedState {
                global_limit,
                kind_limits,
                ..NestedState::default()
            })),
        })
    }

    /// Acquire one child permit atomically: the global limit, the per-kind
    /// limit, and the per-repository serialization are all checked under
    /// one lock in a fixed order, so acquisition cannot deadlock.
    pub fn acquire(
        &self,
        kind: WorkKind,
        repository: impl Into<String>,
    ) -> Result<Option<ChildPermit>, PermitError> {
        let mut state = self.state.lock().expect("nested ledger");
        if state.cancelled {
            return Err(PermitError::RunCancelled);
        }
        let repository = repository.into();
        if state.per_repository.contains_key(&repository) {
            // The repository already holds a stage: the mutation lifecycle
            // is serialized; overlapping stages are refused, not queued.
            return Ok(None);
        }
        if state.global_active >= state.global_limit {
            return Ok(None);
        }
        let kind_limit = state.kind_limits.get(&kind).copied().unwrap_or(usize::MAX);
        if state.kind_active.get(&kind).copied().unwrap_or(0) >= kind_limit {
            return Ok(None);
        }
        *state.kind_active.entry(kind).or_default() += 1;
        state.global_active += 1;
        state.per_repository.insert(repository.clone(), kind);
        Ok(Some(ChildPermit { kind, repository }))
    }

    /// Release a permit.  Cancellation releases a permit only after the
    /// caller confirms its descendants terminated; otherwise the permit
    /// stays held so no new child work can overlap live descendants.
    pub fn release(
        &self,
        permit: &ChildPermit,
        descendants_terminated: bool,
    ) -> Result<(), PermitError> {
        let mut state = self.state.lock().expect("nested ledger");
        match state.per_repository.get(&permit.repository) {
            Some(held_kind) if *held_kind == permit.kind => {}
            _ => {
                return Err(PermitError::NotHeld {
                    repository: permit.repository.clone(),
                });
            }
        }
        if !descendants_terminated {
            return Err(PermitError::DescendantsActive {
                repository: permit.repository.clone(),
            });
        }
        state.per_repository.remove(&permit.repository);
        state.global_active = state.global_active.saturating_sub(1);
        let kind_active = state.kind_active.entry(permit.kind).or_default();
        *kind_active = kind_active.saturating_sub(1);
        Ok(())
    }

    /// The number of repositories currently holding a stage.
    pub fn active_repositories(&self) -> usize {
        self.state
            .lock()
            .expect("nested ledger")
            .per_repository
            .len()
    }

    /// Mark the run cancelled: new acquisition stops.
    pub fn cancel(&self) {
        self.state.lock().expect("nested ledger").cancelled = true;
    }
}

#[cfg(test)]
mod nested_permits_tests;

#[cfg(test)]
mod fairness_tests;
