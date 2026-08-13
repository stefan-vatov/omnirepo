//! Bounded push of only the recorded OID to the selected ref.
//!
//! The transport offers exactly one refspec: `<recorded-oid>:<selected
//! ref>` with tags disabled, so no tag, other branch, or incidental ref is
//! ever sent.  The intent is journaled before contact; response evidence is
//! journaled after.  The child runs in its own process group under a hard
//! deadline; a timeout or cancellation terminates the child and its
//! descendants.  Output is bounded and sanitized; remote rejection is a
//! typed failure.

#![allow(dead_code)]

use super::journal::{JournalError, JournalHandle};
use crate::lifecycle::event::{EvidenceKind, EvidenceRef, JournalEvent, Operation};
use crate::lifecycle::remote_target::FrozenRemoteTarget;
use crate::repository::RevisionId;
use std::{
    error::Error,
    fmt,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Default push deadline.
pub const DEFAULT_PUSH_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum captured push output bytes.
pub const MAX_PUSH_OUTPUT_BYTES: usize = 64 * 1024;

/// The push outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushOutcome {
    pub pushed: bool,
    pub evidence: String,
}

/// Push failures.
#[derive(Debug)]
pub enum PushError {
    Intent { reason: String },
    Spawn { reason: String },
    Timeout { budget: Duration },
    RemoteRejected { reason: String },
    Evidence { reason: String },
    Journal(JournalError),
}

impl fmt::Display for PushError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Intent { reason } => write!(formatter, "push intent failure: {reason}"),
            Self::Spawn { reason } => write!(formatter, "cannot start the push child: {reason}"),
            Self::Timeout { budget } => {
                write!(
                    formatter,
                    "push exceeded its {budget:?} budget and was terminated"
                )
            }
            Self::RemoteRejected { reason } => {
                write!(formatter, "remote rejected the push: {reason}")
            }
            Self::Evidence { reason } => write!(formatter, "push evidence failure: {reason}"),
            Self::Journal(error) => write!(formatter, "push journal failure: {error}"),
        }
    }
}
impl Error for PushError {}

/// Push exactly the recorded OID to the selected ref, journaling intent
/// before contact and response evidence after.  Nothing else is offered to
/// the transport.
pub fn push_recorded_oid(
    working: &crate::platform::AuthorityRoot<
        crate::platform::GitWorkingDirectoryRoot,
        crate::platform::ReadOnly,
    >,
    target: &FrozenRemoteTarget,
    oid: &RevisionId,
    repository_id: &str,
    journal: &JournalHandle,
    run_id: &str,
    timeout: Duration,
) -> Result<PushOutcome, PushError> {
    journal
        .submit(JournalEvent::RepositoryIntent {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: repository_id.to_owned(),
            operation: Operation::Push,
            attempt: 1,
        })
        .map_err(PushError::Journal)?;
    let refspec = format!("{}:{}", oid.as_str(), target.reference.as_str());
    let mut command = sanitized_command(working.display_path().as_path());
    command
        .arg("push")
        .arg("--no-tags")
        .arg("--porcelain")
        .arg(&target.remote)
        .arg(&refspec)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group so descendants die with the push child.
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| PushError::Spawn {
        reason: error.to_string(),
    })?;
    let stdout = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stderr = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut readers = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        readers.push(spawn_reader(pipe, std::sync::Arc::clone(&stdout)));
    }
    if let Some(pipe) = child.stderr.take() {
        readers.push(spawn_reader(pipe, std::sync::Arc::clone(&stderr)));
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    terminate(&mut child);
                    join_readers(readers);
                    return Err(PushError::Timeout { budget: timeout });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                terminate(&mut child);
                join_readers(readers);
                return Err(PushError::Spawn {
                    reason: error.to_string(),
                });
            }
        }
    };
    join_readers(readers);
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stdout.lock().expect("capture")),
        String::from_utf8_lossy(&stderr.lock().expect("capture"))
    );
    let evidence = crate::lifecycle::agent_framing::sanitize_output(&combined);
    if !status.success() {
        return Err(PushError::RemoteRejected {
            reason: evidence.trim().to_owned(),
        });
    }
    journal_evidence(journal, run_id, &evidence)?;
    Ok(PushOutcome {
        pushed: true,
        evidence,
    })
}

/// Journal the response evidence (stage push).
pub(crate) fn journal_evidence(
    journal: &JournalHandle,
    run_id: &str,
    evidence: &str,
) -> Result<(), PushError> {
    let reference = EvidenceRef::new(
        EvidenceKind::Process,
        format!("push/{}", evidence.len()),
        evidence.len() as u64,
    )
    .map_err(|error| PushError::Evidence {
        reason: error.to_string(),
    })?;
    journal
        .submit(JournalEvent::Evidence {
            checkpoint: 0,
            run_id: run_id.to_owned(),
            repository_id: None,
            evidence: reference,
            stage: Some("push"),
        })
        .map_err(PushError::Journal)?;
    Ok(())
}

fn sanitized_command(working: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(working)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_ASKPASS", "/bin/true")
        .env("SSH_ASKPASS", "")
        .arg("--no-optional-locks")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("filter.lfs.smudge=")
        .arg("-c")
        .arg("filter.lfs.clean=")
        .arg("-c")
        .arg("filter.lfs.process=");
    command
}

fn spawn_reader<R: Read + Send + 'static>(
    mut pipe: R,
    sink: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };
            let mut captured = sink.lock().expect("capture sink");
            let room = MAX_PUSH_OUTPUT_BYTES.saturating_sub(captured.len());
            if room > 0 {
                let take = read.min(room);
                captured.extend_from_slice(&buffer[..take]);
            }
        }
    })
}

fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id();
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(format!("-{pid}"))
            .status();
        let _ = child.kill();
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn join_readers(readers: Vec<std::thread::JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.join();
    }
}

#[cfg(test)]
mod remote_push_tests;

#[cfg(test)]
mod remote_push_fixture_tests;
