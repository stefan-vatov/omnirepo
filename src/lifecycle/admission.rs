//! Canonical run admission and repository lease acquisition.
//!
//! One canonical repository cannot hold two mutation leases at once;
//! disjoint repositories admit independently.  Admission waits are bounded
//! (the .38 contract): a lease unavailable within the deadline is denied,
//! never queued indefinitely.  Every admission decision is journaled with
//! the exact repository identity.

#![allow(dead_code)]

#[cfg(test)]
mod admission_concurrency_tests;

use super::journal::{JournalError, JournalHandle};
use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent};
use crate::repository::RepositoryId;
use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(test)]
mod admission_tests;

/// Default bounded wait for a repository lease.
pub const DEFAULT_LEASE_WAIT: Duration = Duration::from_secs(30);
/// Default wait poll interval.
const LEASE_POLL: Duration = Duration::from_millis(10);

/// One acquired lease: the exclusive mutation permit for a repository.
#[derive(Clone, Debug)]
pub struct Lease {
    repository: String,
    token: u64,
    last_seen: Instant,
}

impl Lease {
    pub fn repository(&self) -> &str {
        &self.repository
    }
    pub fn token(&self) -> u64 {
        self.token
    }

    /// Refresh the heartbeat so the owner stays authoritative.
    pub fn heartbeat(&mut self) {
        self.last_seen = Instant::now();
    }
}

/// Admission outcome for one repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Admission {
    /// The caller holds the exclusive mutation lease.
    Admitted,
    /// The lease became available within the bounded wait and was taken.
    AdmittedAfterWait,
    /// The lease was not available within the deadline.
    Denied { reason: String },
}

/// The shared lease table.
#[derive(Clone, Default)]
pub struct LeaseTable {
    inner: Arc<Mutex<BTreeMap<String, LeaseEntry>>>,
}

/// The held lease with its heartbeat.
#[derive(Clone)]
struct LeaseEntry {
    token: u64,
    last_seen: Instant,
}

impl LeaseTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attempt to acquire the mutation lease for one repository within the
    /// bounded wait.  The outcome is journaled before returning.
    pub fn acquire(
        &self,
        journal: &JournalHandle,
        run_id: &str,
        repository: &RepositoryId,
        wait: Duration,
    ) -> Result<(Admission, Option<Lease>), AdmissionError> {
        let deadline = Instant::now() + wait;
        let mut waited = false;
        loop {
            let token = {
                let mut table = self.inner.lock().expect("lease table");
                if !table.contains_key(repository.as_str()) {
                    let token = table.len() as u64 + 1;
                    table.insert(
                        repository.as_str().to_owned(),
                        LeaseEntry {
                            token,
                            last_seen: Instant::now(),
                        },
                    );
                    Some(token)
                } else {
                    None
                }
            };
            if let Some(token) = token {
                let outcome = if waited {
                    Admission::AdmittedAfterWait
                } else {
                    Admission::Admitted
                };
                if let Err(error) = journal_admission(journal, run_id, repository, &outcome) {
                    // A rejected admission must not leave the lease behind:
                    // the table state stays consistent with the journal.
                    self.inner
                        .lock()
                        .expect("lease table")
                        .remove(repository.as_str());
                    return Err(error);
                }
                return Ok((
                    outcome,
                    Some(Lease {
                        repository: repository.as_str().to_owned(),
                        token,
                        last_seen: Instant::now(),
                    }),
                ));
            }
            if Instant::now() >= deadline {
                let outcome = Admission::Denied {
                    reason: "repository lease unavailable within the bounded wait".to_owned(),
                };
                journal_admission(journal, run_id, repository, &outcome)?;
                return Ok((outcome, None));
            }
            waited = true;
            std::thread::sleep(LEASE_POLL);
        }
    }

    /// Release a lease; a missing or foreign token is a typed error.
    pub fn release(&self, lease: &Lease) -> Result<(), AdmissionError> {
        let mut table = self.inner.lock().expect("lease table");
        match table.get(&lease.repository) {
            Some(entry) if entry.token == lease.token => {
                table.remove(&lease.repository);
                Ok(())
            }
            Some(_) => Err(AdmissionError::ForeignLease {
                repository: lease.repository.clone(),
            }),
            None => Err(AdmissionError::MissingLease {
                repository: lease.repository.clone(),
            }),
        }
    }

    /// True when a repository currently holds a mutation lease.
    pub fn is_held(&self, repository: &str) -> bool {
        self.inner
            .lock()
            .expect("lease table")
            .contains_key(repository)
    }

    /// Refresh a lease's heartbeat; a foreign or missing lease fails.
    pub fn heartbeat(&self, lease: &mut Lease) -> Result<(), AdmissionError> {
        let mut table = self.inner.lock().expect("lease table");
        match table.get_mut(&lease.repository) {
            Some(entry) if entry.token == lease.token => {
                entry.last_seen = Instant::now();
                lease.last_seen = Instant::now();
                Ok(())
            }
            Some(_) => Err(AdmissionError::ForeignLease {
                repository: lease.repository.clone(),
            }),
            None => Err(AdmissionError::MissingLease {
                repository: lease.repository.clone(),
            }),
        }
    }

    /// True when a held lease's heartbeat is older than the stale deadline:
    /// the owner is presumed dead and the lease is reclaimable.
    pub fn is_stale(&self, repository: &str, stale_after: Duration) -> bool {
        self.inner
            .lock()
            .expect("lease table")
            .get(repository)
            .map(|entry| entry.last_seen.elapsed() >= stale_after)
            .unwrap_or(false)
    }

    /// Reclaim stale leases, returning the reclaimed repositories.  Only
    /// leases whose heartbeat expired are removed; live leases are never
    /// touched.
    pub fn reclaim_stale(&self, stale_after: Duration) -> Vec<String> {
        let mut table = self.inner.lock().expect("lease table");
        let stale: Vec<String> = table
            .iter()
            .filter(|(_, entry)| entry.last_seen.elapsed() >= stale_after)
            .map(|(repository, _)| repository.clone())
            .collect();
        for repository in &stale {
            table.remove(repository);
        }
        stale
    }
}

/// Admission and lease failures.
#[derive(Debug)]
pub enum AdmissionError {
    Journal(JournalError),
    ForeignLease { repository: String },
    MissingLease { repository: String },
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(formatter, "admission journal failure: {error}"),
            Self::ForeignLease { repository } => {
                write!(formatter, "lease token mismatch for {repository}")
            }
            Self::MissingLease { repository } => {
                write!(formatter, "no lease exists for {repository}")
            }
        }
    }
}
impl Error for AdmissionError {}

fn journal_admission(
    journal: &JournalHandle,
    run_id: &str,
    repository: &RepositoryId,
    outcome: &Admission,
) -> Result<(), AdmissionError> {
    let label = match outcome {
        Admission::Admitted => "admitted",
        Admission::AdmittedAfterWait => "admitted-after-wait",
        Admission::Denied { .. } => "denied",
    };
    let evidence = EvidenceRef::new(
        EvidenceKind::Process,
        format!("admission/{label}/{}", repository.as_str()),
        0,
    )
    .map_err(|error| AdmissionError::Journal(JournalError::Invalid(error)))?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: Some(repository.as_str().to_owned()),
            evidence,
            stage: Some("admission"),
        })
        .map_err(AdmissionError::Journal)?;
    Ok(())
}
