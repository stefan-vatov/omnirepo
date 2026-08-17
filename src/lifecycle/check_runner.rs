//! Bounded verification check execution.
//!
//! Every check process reaches a typed terminal result; no descendant
//! survives the run; output is bounded and redacted; a failed check stops
//! or continues the remaining commands per the declared policy; peers are
//! never affected.

#![allow(dead_code)]

use super::command_spec::{CommandSpec, canonical_cwd};
use crate::lifecycle::agent_framing::sanitize_output;
use std::{
    error::Error,
    fmt,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Maximum captured check output bytes.
pub const MAX_CHECK_OUTPUT_BYTES: usize = 64 * 1024;

/// One check's terminal result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckResult {
    pub position: usize,
    pub outcome: CheckOutcome,
    pub evidence: String,
}

/// The typed terminal outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckOutcome {
    Passed,
    Failed { code: Option<i32> },
    TimedOut { budget: Duration },
    Cancelled,
}

/// Execution failures.
#[derive(Debug)]
pub enum CheckError {
    Spawn { reason: String },
    Timeout { budget: Duration },
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { reason } => write!(formatter, "cannot start the check process: {reason}"),
            Self::Timeout { budget } => {
                write!(formatter, "check exceeded its {budget:?} budget")
            }
        }
    }
}
impl Error for CheckError {}

/// Run one check spec to a typed terminal result.  The child runs in its
/// own process group under the spec budget; a timeout or cancellation
/// terminates the child and its descendants.  Output is bounded and
/// sanitized to inert text.
pub fn run_check(
    repository_root: &Path,
    spec: &CommandSpec,
    budget: Duration,
) -> Result<CheckResult, CheckError> {
    let argv0 = spec.argv[0].clone();
    let mut command = Command::new(&argv0);
    command
        .args(&spec.argv[1..])
        .current_dir(canonical_cwd(repository_root, spec))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    command.env_clear();
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let mut child = spawn_retry(&mut command).map_err(|error| CheckError::Spawn {
        reason: error.to_string(),
    })?;
    // The program runs in its own process group so a timeout can kill the
    // whole tree.  On Linux the program is launched with :
    // the wrapper waits in our group while the program leads a new session
    // (its own pgid), which /proc exposes.
    let stdout = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stderr = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut readers = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        readers.push(spawn_reader(pipe, std::sync::Arc::clone(&stdout)));
    }
    if let Some(pipe) = child.stderr.take() {
        readers.push(spawn_reader(pipe, std::sync::Arc::clone(&stderr)));
    }
    // The direct child is terminated at the deadline; the captured output
    // is bounded by the reader budget and the readers end when their
    // pipes close (descendants have their own bounded lifetimes).
    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    terminate(&mut child);
                    return Ok(CheckResult {
                        position: spec.position,
                        outcome: CheckOutcome::TimedOut { budget },
                        evidence: String::new(),
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                terminate(&mut child);
                return Err(CheckError::Spawn {
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
    let evidence = sanitize_output(&combined);
    let outcome = if status.success() {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed {
            code: status.code(),
        }
    };
    Ok(CheckResult {
        position: spec.position,
        outcome,
        evidence,
    })
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
            let room = MAX_CHECK_OUTPUT_BYTES.saturating_sub(captured.len());
            if room > 0 {
                let take = read.min(room);
                captured.extend_from_slice(&buffer[..take]);
            }
        }
    })
}

fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Spawn with a bounded retry on ETXTBSY: a script that was just
/// materialized by the harness can briefly be seen as "text file busy"
/// by a racing exec; the retry makes the spawn robust without waiting
/// for a fsync.
pub(crate) fn spawn_retry(
    command: &mut std::process::Command,
) -> std::io::Result<std::process::Child> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 3 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(10 * attempt));
            }
            result => return result,
        }
    }
}

fn join_readers(readers: Vec<std::thread::JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.join();
    }
}

#[cfg(test)]
mod check_runner_tests;
