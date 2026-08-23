//! Re-run the normative gates and verify candidate provenance.
//!
//! The gate orchestrator runs every configured gate over the exact-SHA
//! candidate and collects each result (a failing gate never stops the
//! others).  Provenance verification ties the candidate manifest to the
//! checkout: the manifest's source commit must equal the checkout HEAD
//! and the manifest's content hash must match its own identity — a
//! tampered manifest is refused.

#![allow(dead_code)]

use crate::lifecycle::release_manifest::{CandidateManifest, content_hash};

#[cfg(test)]
mod release_gates_tests;
use std::{
    error::Error,
    fmt,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_GATE_OUTPUT_BYTES: usize = 1024 * 1024;

/// One gate result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateRun {
    pub name: String,
    pub passed: bool,
    pub evidence: String,
}

/// Provenance failures.
#[derive(Debug)]
pub enum ProvenanceError {
    HeadUnavailable {
        path: std::path::PathBuf,
        reason: String,
    },
    CommitMismatch {
        expected: String,
        actual: String,
    },
    ManifestTampered {
        expected: String,
        actual: String,
    },
}

impl fmt::Display for ProvenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeadUnavailable { path, reason } => {
                write!(
                    formatter,
                    "checkout head unavailable {}: {reason}",
                    path.display()
                )
            }
            Self::CommitMismatch { expected, actual } => write!(
                formatter,
                "the checkout HEAD {actual} does not match the manifest commit {expected}"
            ),
            Self::ManifestTampered { expected, actual } => write!(
                formatter,
                "the manifest content hash {actual} does not match its identity {expected}"
            ),
        }
    }
}
impl Error for ProvenanceError {}

/// Run every configured gate over the candidate.  A failing gate is
/// collected, never stopping the others.  Gates are explicit argument
/// arrays — never shell strings.
pub fn run_normative_gates(gates: &[(String, Vec<String>)]) -> Vec<GateRun> {
    run_normative_gates_with_budget(
        gates,
        crate::lifecycle::command_spec::DEFAULT_COMMAND_TIMEOUT,
    )
}

fn run_normative_gates_with_budget(
    gates: &[(String, Vec<String>)],
    budget: Duration,
) -> Vec<GateRun> {
    gates
        .iter()
        .map(|(name, argv)| run_gate(name, argv, budget))
        .collect()
}

#[derive(Default)]
struct GateCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    overflowed: bool,
}

#[derive(Clone, Copy)]
enum GateStream {
    Stdout,
    Stderr,
}

fn run_gate(name: &str, argv: &[String], budget: Duration) -> GateRun {
    // The bounded ETXTBSY retry (the shared check-runner pattern) makes
    // spawning a just-materialized gate executable robust.
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
    }
    let mut child = match crate::lifecycle::check_runner::spawn_retry(&mut command) {
        Ok(child) => child,
        Err(error) => {
            return failed_gate(name, format!("cannot start gate: {error}"));
        }
    };
    let capture = std::sync::Arc::new(std::sync::Mutex::new(GateCapture::default()));
    let mut readers = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        readers.push(spawn_gate_reader(
            pipe,
            std::sync::Arc::clone(&capture),
            GateStream::Stdout,
        ));
    }
    if let Some(pipe) = child.stderr.take() {
        readers.push(spawn_gate_reader(
            pipe,
            std::sync::Arc::clone(&capture),
            GateStream::Stderr,
        ));
    }
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_gate_process_group(child.id());
                join_gate_readers(readers);
                let captured = capture.lock().expect("gate capture");
                if captured.overflowed {
                    return failed_gate(name, "gate output exceeded the bound".to_owned());
                }
                let stdout = String::from_utf8_lossy(&captured.stdout);
                let stderr = String::from_utf8_lossy(&captured.stderr);
                return GateRun {
                    name: name.to_owned(),
                    passed: status.success(),
                    evidence: format!("{stdout}{stderr}"),
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    terminate_gate(&mut child);
                    join_gate_readers(readers);
                    return failed_gate(name, format!("gate exceeded its {budget:?} budget"));
                }
                if capture.lock().expect("gate capture").overflowed {
                    terminate_gate(&mut child);
                    join_gate_readers(readers);
                    return failed_gate(name, "gate output exceeded the bound".to_owned());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                terminate_gate(&mut child);
                join_gate_readers(readers);
                return failed_gate(name, format!("cannot collect gate output: {error}"));
            }
        }
    }
}

fn failed_gate(name: &str, evidence: String) -> GateRun {
    GateRun {
        name: name.to_owned(),
        passed: false,
        evidence,
    }
}

fn spawn_gate_reader<R: Read + Send + 'static>(
    mut pipe: R,
    capture: std::sync::Arc<std::sync::Mutex<GateCapture>>,
    stream: GateStream,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };
            let mut captured = capture.lock().expect("gate capture");
            let admitted = MAX_GATE_OUTPUT_BYTES
                .saturating_sub(captured.stdout.len().saturating_add(captured.stderr.len()));
            let take = read.min(admitted);
            let destination = match stream {
                GateStream::Stdout => &mut captured.stdout,
                GateStream::Stderr => &mut captured.stderr,
            };
            destination.extend_from_slice(&buffer[..take]);
            if take < read {
                captured.overflowed = true;
                break;
            }
        }
    })
}

fn terminate_gate(child: &mut std::process::Child) {
    terminate_gate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn terminate_gate_process_group(process_group: u32) {
    unsafe {
        let _ = killpg(process_group as i32, SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate_gate_process_group(_process_group: u32) {}

fn join_gate_readers(readers: Vec<std::thread::JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.join();
    }
}

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn killpg(process_group: i32, signal: i32) -> i32;
}

/// Verify the candidate provenance against the checkout: the manifest's
/// source commit equals the checkout HEAD, and the manifest's content
/// hash matches its own identity (no tampering).
pub fn verify_candidate_provenance(
    manifest: &CandidateManifest,
    checkout: &Path,
) -> Result<(), ProvenanceError> {
    let head_file = checkout.join("HEAD");
    let head =
        std::fs::read_to_string(&head_file).map_err(|error| ProvenanceError::HeadUnavailable {
            path: head_file.clone(),
            reason: error.to_string(),
        })?;
    let head = head.trim().to_owned();
    if head != manifest.identity.source_commit {
        return Err(ProvenanceError::CommitMismatch {
            expected: manifest.identity.source_commit.clone(),
            actual: head,
        });
    }
    let expected = content_hash(manifest);
    if expected != manifest.identity.manifest_sha256 {
        return Err(ProvenanceError::ManifestTampered {
            expected,
            actual: manifest.identity.manifest_sha256.clone(),
        });
    }
    Ok(())
}
