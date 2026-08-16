//! Agent evidence capture and crash/timeout/descendant termination.
//!
//! Repair agents run under the destination-only confinement with a bounded
//! output budget; stdout is captured, sanitized, and written as evidence
//! beneath the destination.  A crash (non-zero exit) and a timeout both
//! terminate the agent; on Unix the agent runs in its own process group so
//! descendants are reaped with it.

#![allow(dead_code)]

use super::agent_confinement::AgentConfinement;
use super::agent_framing::{MAX_FRAME_PAYLOAD_BYTES, sanitize_output};
/// Spawn with a bounded retry on ETXTBSY: a script that was just
/// materialized by the harness can briefly be seen as "text file busy"
/// by a racing exec; the retry makes the spawn robust without waiting
/// for a fsync.
fn spawn_retry(command: &mut Command) -> std::io::Result<std::process::Child> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 3 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(10 * attempt));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod agent_runtime_tests;

use std::{
    error::Error,
    fmt,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Default agent run budget: fifteen minutes
/// (canon/architecture/fleet-lifecycle.md: agent-assisted repair).
pub const DEFAULT_AGENT_TIMEOUT: Duration = Duration::from_secs(900);
/// Maximum captured evidence bytes (post-sanitization).
pub const MAX_EVIDENCE_BYTES: usize = MAX_FRAME_PAYLOAD_BYTES;

/// The captured agent evidence.
#[derive(Clone, Debug)]
pub struct AgentEvidence {
    pub evidence_path: std::path::PathBuf,
    pub sanitized: String,
}

/// Agent runtime failures.
#[derive(Debug)]
pub enum AgentRuntimeError {
    Spawn {
        reason: String,
    },
    Timeout {
        budget: Duration,
    },
    Crashed {
        code: Option<i32>,
    },
    Evidence {
        path: std::path::PathBuf,
        reason: String,
    },
}

impl fmt::Display for AgentRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { reason } => write!(formatter, "cannot start repair agent: {reason}"),
            Self::Timeout { budget } => {
                write!(formatter, "repair agent exceeded its {budget:?} budget")
            }
            Self::Crashed { code } => write!(formatter, "repair agent crashed with code {code:?}"),
            Self::Evidence { path, reason } => {
                write!(
                    formatter,
                    "cannot capture agent evidence {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for AgentRuntimeError {}

/// Run the agent with its argv under the confinement; capture sanitized
/// evidence and enforce the budget with descendant reaping.
pub fn run_agent(
    argv: &[String],
    confinement: &AgentConfinement,
    evidence_dir: &Path,
    timeout: Duration,
) -> Result<AgentEvidence, AgentRuntimeError> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(&confinement.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &confinement.env {
        command.env(key, value);
    }
    // The confined environment replaces the ambient one entirely: no
    // credential or ambient variable survives into the agent.
    command.env_clear();
    for (key, value) in &confinement.env {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group so descendants die with the agent.
        command.process_group(0);
    }
    let mut child = spawn_retry(&mut command).map_err(|error| AgentRuntimeError::Spawn {
        reason: error.to_string(),
    })?;
    // Reader threads drain both pipes to EOF; they stop accepting bytes at
    // the evidence budget and discard the remainder so a verbose agent can
    // finish instead of blocking on a full pipe.
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
                    return Err(AgentRuntimeError::Timeout { budget: timeout });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                terminate(&mut child);
                join_readers(readers);
                return Err(AgentRuntimeError::Spawn {
                    reason: error.to_string(),
                });
            }
        }
    };
    join_readers(readers);
    if !status.success() {
        return Err(AgentRuntimeError::Crashed {
            code: status.code(),
        });
    }
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stdout.lock().expect("capture")),
        String::from_utf8_lossy(&stderr.lock().expect("capture"))
    );
    let sanitized = sanitize_output(&combined);
    std::fs::create_dir_all(evidence_dir).map_err(|error| AgentRuntimeError::Evidence {
        path: evidence_dir.to_path_buf(),
        reason: error.to_string(),
    })?;
    let evidence_path = evidence_dir.join("agent-evidence.txt");
    std::fs::write(&evidence_path, &sanitized).map_err(|error| AgentRuntimeError::Evidence {
        path: evidence_path.clone(),
        reason: error.to_string(),
    })?;
    Ok(AgentEvidence {
        evidence_path,
        sanitized,
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
            let room = MAX_EVIDENCE_BYTES.saturating_sub(captured.len());
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
        // The agent runs in its own process group (process_group(0)), so a
        // group signal reaps descendants with the agent: TERM first (the
        // graceful boundary), then SIGKILL for anything that ignored it.
        let pid = child.id();
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg("--")
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
