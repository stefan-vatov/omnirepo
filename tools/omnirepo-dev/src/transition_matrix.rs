//! Isolated Beads lifecycle transition evidence.
//!
//! This module is test tooling, not product authority. It copies the tracked
//! Beads export into a fresh temporary workspace, exercises the real `br`
//! lifecycle, and returns a bounded, structured report. The live checkout is
//! read only. In particular, actor metadata is recorded as cooperative
//! provenance; it is never treated as owner authentication.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

pub const TRANSITION_MATRIX_SCHEMA: &str = "omnirepo.beads-transition-matrix.v1";

/// Stable case order copied from the original shell matrix.
pub const CASE_IDS: [&str; 13] = [
    "active-decision",
    "owner-close",
    "raw-reopen",
    "reopen-restore",
    "invalid-claim",
    "claim-restored",
    "invalid-relabel",
    "relabel-restored",
    "ordinary-status-drift",
    "ordinary-label-drift",
    "ordinary-work",
    "stale-export",
    "stale-restored",
];

const DECISION_LABELS: [&str; 2] = ["decision-needed", "human-input"];
const EMPTY_LABELS: [&str; 0] = [];
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// The bound on waiting for a terminated process tree to disappear.
const REAP_TIMEOUT: Duration = Duration::from_secs(10);
/// The poll interval used while waiting for that disappearance.
const REAP_POLL_INTERVAL: Duration = Duration::from_millis(1);
const MAX_DIAGNOSTIC_BYTES: usize = 4096;
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
static WORKSPACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One matrix outcome. Unsafe tool transitions are expected evidence: the
/// cooperative policy sees the drift and the fixture is restored before the
/// next case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaseOutcome {
    Pass,
    ToolRejected,
    UnsafeToolTransition,
}

impl CaseOutcome {
    pub fn is_success(self) -> bool {
        matches!(
            self,
            Self::Pass | Self::ToolRejected | Self::UnsafeToolTransition
        )
    }
}

/// Structured evidence for one frozen transition case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseReport {
    pub case_id: String,
    pub operation: String,
    pub expected: Value,
    pub observed: Value,
    pub outcome: CaseOutcome,
    pub evidence: String,
}

/// The complete matrix report. Live hashes cover only the stable tracked
/// JSONL export and are diagnostic evidence, not a correctness gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixReport {
    pub schema: &'static str,
    pub cases: Vec<CaseReport>,
    pub live_before_hash: String,
    pub live_after_hash: String,
    /// Diagnostic comparison of the stable tracked export before and after.
    /// This is intentionally not a correctness dependency under live writes.
    pub live_snapshot_unchanged: bool,
    pub workspace_removed: bool,
}

/// Errors are explicit so a missing or incompatible tracker fails the gate;
/// the test never treats an unavailable `br` as a skipped case.
#[derive(Debug)]
pub enum MatrixError {
    InvalidRepository {
        path: PathBuf,
        reason: &'static str,
    },
    LiveBeadsMissing {
        path: PathBuf,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    MissingBr {
        path: PathBuf,
    },
    BrProbeFailed {
        path: PathBuf,
        code: Option<i32>,
        stderr: String,
    },
    BrFailed {
        operation: String,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    InvalidJson {
        operation: String,
        detail: String,
    },
    CaseFailed {
        case_id: String,
        detail: String,
    },
    CleanupFailed {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for MatrixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepository { path, reason } => {
                write!(
                    formatter,
                    "transition matrix repository {}: {reason}",
                    path.display()
                )
            }
            Self::LiveBeadsMissing { path } => {
                write!(
                    formatter,
                    "transition matrix requires tracked Beads export: {}",
                    path.display()
                )
            }
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(
                    formatter,
                    "transition matrix could not {operation} {}: {source}",
                    path.display()
                )
            }
            Self::MissingBr { path } => {
                write!(
                    formatter,
                    "transition matrix requires an executable br: {}",
                    path.display()
                )
            }
            Self::BrProbeFailed { path, code, stderr } => {
                write!(
                    formatter,
                    "br probe failed for {} (exit {:?}): {}",
                    path.display(),
                    code,
                    stderr
                )
            }
            Self::BrFailed {
                operation,
                code,
                stdout,
                stderr,
            } => {
                write!(
                    formatter,
                    "br {operation} failed (exit {:?}): {stderr} {stdout}",
                    code
                )
            }
            Self::InvalidJson { operation, detail } => {
                write!(
                    formatter,
                    "transition matrix JSON from {operation} is invalid: {detail}"
                )
            }
            Self::CaseFailed { case_id, detail } => {
                write!(
                    formatter,
                    "transition matrix case {case_id} failed: {detail}"
                )
            }
            Self::CleanupFailed { path, source } => {
                write!(
                    formatter,
                    "transition matrix could not remove {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for MatrixError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::CleanupFailed { source, .. } => Some(source),
            Self::InvalidRepository { .. }
            | Self::LiveBeadsMissing { .. }
            | Self::MissingBr { .. }
            | Self::BrProbeFailed { .. }
            | Self::BrFailed { .. }
            | Self::InvalidJson { .. }
            | Self::CaseFailed { .. } => None,
        }
    }
}

/// Run the matrix using the `br` found through the current process PATH.
pub fn run(repository_root: &Path) -> Result<MatrixReport, MatrixError> {
    run_with_br_path_and_timeout(
        repository_root,
        PathBuf::from("br"),
        DEFAULT_COMMAND_TIMEOUT,
    )
}

/// Run the matrix with an explicit tracker executable. This seam makes
/// unavailable and non-zero tracker probes testable without changing PATH.
pub fn run_with_br_path(
    repository_root: &Path,
    br_path: PathBuf,
) -> Result<MatrixReport, MatrixError> {
    run_with_br_path_and_timeout(repository_root, br_path, DEFAULT_COMMAND_TIMEOUT)
}

/// Run the matrix with an explicit command timeout.
pub fn run_with_br_path_and_timeout(
    repository_root: &Path,
    br_path: PathBuf,
    timeout: Duration,
) -> Result<MatrixReport, MatrixError> {
    run_with_br_path_and_timeout_hook(repository_root, br_path, timeout, None)
}

fn run_with_br_path_and_timeout_hook(
    repository_root: &Path,
    br_path: PathBuf,
    timeout: Duration,
    after_before_snapshot: Option<&dyn Fn(&Path)>,
) -> Result<MatrixReport, MatrixError> {
    let live_beads = repository_root.join(".beads");
    let live_jsonl = live_beads.join("issues.jsonl");
    validate_repository(repository_root, &live_jsonl)?;
    let before = LiveSnapshot::capture(&live_beads)?;
    if let Some(hook) = after_before_snapshot {
        hook(&live_beads);
    }
    let program = BrProgram::probe(br_path, timeout)?;

    let workspace = TempWorkspace::create(repository_root, &program)?;
    let workspace_root = workspace.root.clone();
    let matrix_result = execute_matrix(&program, &workspace);
    let cleanup_result = workspace.cleanup();
    cleanup_result?;
    let after = LiveSnapshot::capture(&live_beads)?;
    let before_hash = before.digest();
    let after_hash = after.digest();

    let mut report = matrix_result?;
    report.live_before_hash = before_hash;
    report.live_after_hash = after_hash;
    report.live_snapshot_unchanged = before == after;
    report.workspace_removed = !workspace_root.exists();
    if !report.workspace_removed {
        return Err(MatrixError::CleanupFailed {
            path: workspace_root,
            source: io::Error::other("workspace still exists after cleanup"),
        });
    }
    Ok(report)
}

fn validate_repository(repository_root: &Path, live_jsonl: &Path) -> Result<(), MatrixError> {
    if !repository_root.is_dir() {
        return Err(MatrixError::InvalidRepository {
            path: repository_root.to_path_buf(),
            reason: "root is not a directory",
        });
    }
    if !live_jsonl.is_file() {
        return Err(MatrixError::LiveBeadsMissing {
            path: live_jsonl.to_path_buf(),
        });
    }
    Ok(())
}

struct BrProgram {
    path: PathBuf,
    timeout: Duration,
}

impl BrProgram {
    fn probe(path: PathBuf, timeout: Duration) -> Result<Self, MatrixError> {
        let mut command = Command::new(&path);
        // Probe outside the repository so even discovery-related br behavior
        // cannot select or mutate the live `.beads` state.
        command.arg("--version").current_dir(std::env::temp_dir());
        let output = capture_command(command, timeout).map_err(|error| match error {
            CaptureFailure::Spawn(source) if source.kind() == io::ErrorKind::NotFound => {
                MatrixError::MissingBr { path: path.clone() }
            }
            other => MatrixError::BrProbeFailed {
                path: path.clone(),
                code: other.code(),
                stderr: other.diagnostic(),
            },
        })?;
        if !output.status.success() {
            return Err(MatrixError::BrProbeFailed {
                path: path.clone(),
                code: output.status.code(),
                stderr: bounded_bytes(&output.stderr),
            });
        }
        Ok(Self { path, timeout })
    }

    fn command(
        &self,
        workspace: &Path,
        db: Option<&Path>,
        args: &[&str],
        no_flush: bool,
    ) -> Result<BrOutput, MatrixError> {
        let mut command = Command::new(&self.path);
        command.current_dir(workspace);
        if let Some(db) = db {
            command.arg("--db").arg(db).arg("--no-auto-import");
            if no_flush {
                command.arg("--no-auto-flush");
            }
        }
        command.args(args);
        let operation = args.join(" ");
        let output = capture_command(command, self.timeout)
            .map_err(|error| error.into_matrix_error(operation.clone()))?;
        admit_output(args.join(" "), BrOutput::from_output(output))
    }

    fn must_command(
        &self,
        workspace: &Path,
        db: &Path,
        args: &[&str],
    ) -> Result<BrOutput, MatrixError> {
        let operation = args.join(" ");
        let result = self.command(workspace, Some(db), args, false)?;
        if result.success() {
            Ok(result)
        } else {
            Err(result.into_failure(operation))
        }
    }

    fn must_command_no_flush(
        &self,
        workspace: &Path,
        db: &Path,
        args: &[&str],
    ) -> Result<BrOutput, MatrixError> {
        let operation = args.join(" ");
        let result = self.command(workspace, Some(db), args, true)?;
        if result.success() {
            Ok(result)
        } else {
            Err(result.into_failure(operation))
        }
    }
}

#[derive(Debug)]
enum CaptureFailure {
    Spawn(io::Error),
    Wait {
        reason: String,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Timeout {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    OutputTooLarge {
        stream: &'static str,
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    MissingPipe(&'static str),
}

impl CaptureFailure {
    fn code(&self) -> Option<i32> {
        match self {
            Self::OutputTooLarge { code, .. } => *code,
            Self::Spawn(_) | Self::Wait { .. } | Self::Timeout { .. } | Self::MissingPipe(_) => {
                None
            }
        }
    }

    fn diagnostic(&self) -> String {
        match self {
            Self::Spawn(source) => bounded_text(&source.to_string()),
            Self::Wait {
                reason,
                stdout,
                stderr,
            } => bounded_diagnostics(&format!("wait failed: {reason}"), stdout, stderr),
            Self::Timeout { stdout, stderr } => {
                bounded_diagnostics("command timed out", stdout, stderr)
            }
            Self::OutputTooLarge {
                stream,
                stdout,
                stderr,
                ..
            } => bounded_diagnostics(
                &format!("{stream} exceeded {MAX_OUTPUT_BYTES} bytes"),
                stdout,
                stderr,
            ),
            Self::MissingPipe(stream) => format!("{stream} pipe was not created"),
        }
    }

    fn into_matrix_error(self, operation: String) -> MatrixError {
        let code = self.code();
        let (stdout, stderr, detail) = match self {
            Self::Spawn(source) => (String::new(), bounded_text(&source.to_string()), None),
            Self::Wait {
                reason,
                stdout,
                stderr,
            } => (
                bounded_bytes(&stdout),
                bounded_diagnostics(&format!("wait failed: {reason}"), &[], &stderr),
                None,
            ),
            Self::Timeout { stdout, stderr } => (
                bounded_bytes(&stdout),
                bounded_diagnostics("command timed out", &[], &stderr),
                None,
            ),
            Self::OutputTooLarge {
                stream,
                stdout,
                stderr,
                ..
            } => (
                bounded_bytes(&stdout),
                bounded_diagnostics(
                    &format!("{stream} exceeded {MAX_OUTPUT_BYTES} bytes"),
                    &[],
                    &stderr,
                ),
                Some(stream),
            ),
            Self::MissingPipe(stream) => (
                String::new(),
                format!("{stream} pipe was not created"),
                None,
            ),
        };
        let stdout = if let Some(stream) = detail {
            format!("{stream} exceeded transition matrix output bound: {stdout}")
        } else {
            stdout
        };
        MatrixError::BrFailed {
            operation,
            code,
            stdout: bounded_text(&stdout),
            stderr: bounded_text(&stderr),
        }
    }
}

fn bounded_diagnostics(prefix: &str, stdout: &[u8], stderr: &[u8]) -> String {
    let mut detail = bounded_text(prefix);
    let stderr = bounded_bytes(stderr);
    let stdout = bounded_bytes(stdout);
    if !stderr.is_empty() {
        detail.push_str("; stderr=");
        detail.push_str(&stderr);
    }
    if !stdout.is_empty() {
        detail.push_str("; stdout=");
        detail.push_str(&stdout);
    }
    bounded_text(&detail)
}

#[derive(Debug)]
struct ReaderResult {
    bytes: Vec<u8>,
    overflowed: bool,
}

struct ReaderHandle {
    overflowed: Arc<std::sync::atomic::AtomicBool>,
    join: Option<JoinHandle<ReaderResult>>,
}

impl ReaderHandle {
    fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }

    fn join(mut self) -> ReaderResult {
        self.join
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or(ReaderResult {
                bytes: Vec::new(),
                overflowed: self.overflowed(),
            })
    }
}

fn spawn_reader<R>(mut reader: R, limit: usize) -> ReaderHandle
where
    R: Read + Send + 'static,
{
    let overflowed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let marker = Arc::clone(&overflowed);
    let join = thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(8192));
        let mut chunk = [0_u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    return ReaderResult {
                        bytes,
                        overflowed: marker.load(Ordering::Acquire),
                    };
                }
                Ok(read) => {
                    let remaining = limit.saturating_sub(bytes.len());
                    if read > remaining {
                        bytes.extend_from_slice(&chunk[..remaining]);
                        marker.store(true, Ordering::Release);
                        return ReaderResult {
                            bytes,
                            overflowed: true,
                        };
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    return ReaderResult {
                        bytes,
                        overflowed: marker.load(Ordering::Acquire),
                    };
                }
            }
        }
    });
    ReaderHandle {
        overflowed,
        join: Some(join),
    }
}

fn capture_command(mut command: Command, timeout: Duration) -> Result<Output, CaptureFailure> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);
    let mut child = command.spawn().map_err(CaptureFailure::Spawn)?;
    let pid = child.id();
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_process_tree(pid);
            let _ = child.wait();
            return Err(CaptureFailure::MissingPipe("stdout"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_process_tree(pid);
            let _ = child.wait();
            return Err(CaptureFailure::MissingPipe("stderr"));
        }
    };
    let stdout_reader = spawn_reader(stdout, MAX_OUTPUT_BYTES);
    let stderr_reader = spawn_reader(stderr, MAX_OUTPUT_BYTES);
    let (status_sender, status_receiver) = mpsc::channel();
    let waiter = thread::spawn(move || status_sender.send(child.wait()));

    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut timed_out = false;
    let mut wait_failed = None;
    loop {
        if stdout_reader.overflowed() || stderr_reader.overflowed() {
            terminate_process_tree(pid);
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timed_out = true;
            terminate_process_tree(pid);
            break;
        }
        match status_receiver.recv_timeout(remaining.min(PROCESS_POLL_INTERVAL)) {
            Ok(Ok(exit_status)) => {
                status = Some(exit_status);
                // A direct child can leave descendants holding our pipes open.
                // The process group is isolated, so reap those descendants
                // before joining the bounded readers.
                terminate_process_tree(pid);
                break;
            }
            Ok(Err(error)) => {
                wait_failed = Some(error.to_string());
                terminate_process_tree(pid);
                break;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                wait_failed = Some("waiter channel disconnected".to_owned());
                terminate_process_tree(pid);
                break;
            }
        }
    }

    if status.is_none() && wait_failed.is_none() {
        match status_receiver.recv() {
            Ok(Ok(exit_status)) => status = Some(exit_status),
            Ok(Err(error)) => wait_failed = Some(error.to_string()),
            Err(_) => wait_failed = Some("waiter channel disconnected".to_owned()),
        }
    }
    let _ = waiter.join();
    // Every path out of the loop above signalled the tree.  The direct
    // child is reaped by the joined waiter; this awaits the descendants
    // so the command tree is gone before any result is returned and the
    // bounded readers can no longer be held open by a live writer.
    await_process_tree_reaped(pid);
    let stdout_result = stdout_reader.join();
    let stderr_result = stderr_reader.join();

    if timed_out {
        return Err(CaptureFailure::Timeout {
            stdout: stdout_result.bytes,
            stderr: stderr_result.bytes,
        });
    }
    if stdout_result.overflowed {
        return Err(CaptureFailure::OutputTooLarge {
            stream: "stdout",
            code: status.as_ref().and_then(ExitStatus::code),
            stdout: stdout_result.bytes,
            stderr: stderr_result.bytes,
        });
    }
    if stderr_result.overflowed {
        return Err(CaptureFailure::OutputTooLarge {
            stream: "stderr",
            code: status.as_ref().and_then(ExitStatus::code),
            stdout: stdout_result.bytes,
            stderr: stderr_result.bytes,
        });
    }
    if let Some(reason) = wait_failed {
        return Err(CaptureFailure::Wait {
            reason,
            stdout: stdout_result.bytes,
            stderr: stderr_result.bytes,
        });
    }
    let Some(status) = status else {
        return Err(CaptureFailure::Wait {
            reason: "child status was unavailable after wait".to_owned(),
            stdout: stdout_result.bytes,
            stderr: stderr_result.bytes,
        });
    };
    Ok(Output {
        status,
        stdout: stdout_result.bytes,
        stderr: stderr_result.bytes,
    })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: `pre_exec` runs in the child between fork and exec. It only
    // creates a private process group for this command tree.
    unsafe {
        command.pre_exec(|| {
            if setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) {
    let pid = pid as i32;
    // SAFETY: `configure_process_group` created this command's private group;
    // the direct-child fallback covers an unavailable group at termination.
    unsafe {
        let _ = kill(-pid, SIGKILL);
        let _ = kill(pid, SIGKILL);
    }
}

/// Wait until the command's private process group holds no process.
///
/// `SIGKILL` delivery is asynchronous: `kill` returns once the signal is
/// queued, while the kernel still tears the processes down and reaps the
/// descendants that outlived their killed parent.  Returning at signal
/// time therefore leaves observable live descendants, so termination is
/// completed here: the group is polled until it is empty, which is the
/// point at which the tree is both terminated and reaped.  The wait is
/// bounded; an exhausted bound returns rather than blocking the run.
#[cfg(unix)]
fn await_process_tree_reaped(pid: u32) {
    let pid = pid as i32;
    let deadline = Instant::now() + REAP_TIMEOUT;
    loop {
        // SAFETY: signal 0 performs only the existence and permission
        // check for the private group; it delivers no signal.  The group
        // id is this command's own child pid, so no unrelated group can
        // be observed while any member of it still exists.
        let present = unsafe { kill(-pid, 0) } == 0;
        if !present || Instant::now() >= deadline {
            return;
        }
        thread::sleep(REAP_POLL_INTERVAL);
    }
}

#[cfg(not(unix))]
fn await_process_tree_reaped(_pid: u32) {}

#[cfg(windows)]
fn terminate_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(_pid: u32) {}

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn setpgid(pid: i32, pgid: i32) -> i32;
}

#[derive(Debug, Clone)]
struct BrOutput {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl BrOutput {
    fn from_output(output: Output) -> Self {
        Self {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }

    #[cfg(test)]
    fn from_parts(code: Option<i32>, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            code,
            stdout,
            stderr,
        }
    }

    fn success(&self) -> bool {
        self.code == Some(0)
    }

    fn into_failure(self, operation: String) -> MatrixError {
        MatrixError::BrFailed {
            operation,
            code: self.code,
            stdout: bounded_bytes(&self.stdout),
            stderr: bounded_bytes(&self.stderr),
        }
    }
}

fn admit_output(operation: String, output: BrOutput) -> Result<BrOutput, MatrixError> {
    if output.stdout.len() > MAX_OUTPUT_BYTES || output.stderr.len() > MAX_OUTPUT_BYTES {
        return Err(MatrixError::BrFailed {
            operation,
            code: output.code,
            stdout: "output exceeded the transition matrix bound".to_owned(),
            stderr: String::new(),
        });
    }
    Ok(output)
}

struct TempWorkspaceGuard {
    root: Option<PathBuf>,
}

impl TempWorkspaceGuard {
    fn new(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }

    fn path(&self) -> &Path {
        self.root
            .as_deref()
            .expect("temporary workspace guard must own a root")
    }

    fn disarm(&mut self) {
        self.root = None;
    }
}

impl Drop for TempWorkspaceGuard {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let _ = fs::remove_dir_all(root);
        }
    }
}

struct TempWorkspace {
    root: PathBuf,
    db: PathBuf,
    cleaned: bool,
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

impl TempWorkspace {
    fn create(repository_root: &Path, program: &BrProgram) -> Result<Self, MatrixError> {
        let fixture = FrozenBeadsFixture::read(&repository_root.join(".beads"))?;
        let root = unique_temp_directory()?;
        let mut guard = TempWorkspaceGuard::new(root);
        let init = program.command(guard.path(), None, &["init", "--prefix", "omni"], false)?;
        if !init.success() {
            return Err(init.into_failure("init --prefix omni".to_owned()));
        }

        let sandbox_beads = guard.path().join(".beads");
        fixture.write(&sandbox_beads)?;
        let root = guard.path().to_owned();
        let db = sandbox_beads.join("beads.db");
        let workspace = Self {
            root,
            db,
            cleaned: false,
        };
        workspace.import_rebuild(program)?;
        guard.disarm();
        Ok(workspace)
    }

    fn import_rebuild(&self, program: &BrProgram) -> Result<(), MatrixError> {
        program.must_command(
            &self.root,
            &self.db,
            &["sync", "--import-only", "--rebuild"],
        )?;
        Ok(())
    }

    fn cleanup(mut self) -> Result<(), MatrixError> {
        let path = self.root.clone();
        let result =
            fs::remove_dir_all(&path).map_err(|source| MatrixError::CleanupFailed { path, source });
        if result.is_ok() {
            self.cleaned = true;
        }
        result
    }
}

/// Inputs for the matrix are frozen before any `br` command starts. This
/// prevents a concurrent live tracker write from changing the fixture.
struct FrozenBeadsFixture {
    files: BTreeMap<String, Vec<u8>>,
}

impl FrozenBeadsFixture {
    fn read(root: &Path) -> Result<Self, MatrixError> {
        let mut files = BTreeMap::new();
        for name in [
            "issues.jsonl",
            "policy.yaml",
            "config.yaml",
            "metadata.json",
        ] {
            let path = root.join(name);
            if path.is_file() {
                let bytes = fs::read(&path).map_err(|source| MatrixError::Io {
                    operation: "freeze Beads fixture",
                    path: path.clone(),
                    source,
                })?;
                files.insert(name.to_owned(), bytes);
            }
        }
        Ok(Self { files })
    }

    fn write(&self, root: &Path) -> Result<(), MatrixError> {
        for (name, bytes) in &self.files {
            let path = root.join(name);
            fs::write(&path, bytes).map_err(|source| MatrixError::Io {
                operation: "write frozen Beads fixture",
                path,
                source,
            })?;
        }
        Ok(())
    }
}

fn unique_temp_directory() -> Result<PathBuf, MatrixError> {
    let base = std::env::temp_dir();
    for _ in 0..128 {
        let sequence = WORKSPACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "omnirepo-transition-matrix-{}-{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(MatrixError::Io {
                    operation: "create temporary workspace",
                    path,
                    source,
                });
            }
        }
    }
    Err(MatrixError::Io {
        operation: "allocate unique temporary workspace",
        path: base,
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "workspace allocation limit reached",
        ),
    })
}

fn execute_matrix(
    program: &BrProgram,
    workspace: &TempWorkspace,
) -> Result<MatrixReport, MatrixError> {
    let decision_id = create_issue(
        program,
        workspace,
        &[
            "create",
            "--title",
            "transition matrix owner decision fixture",
            "--slug",
            "transition-matrix-owner-decision",
            "--labels",
            "decision-needed,human-input",
            "--json",
        ],
    )?;
    let dependent_id = create_issue(
        program,
        workspace,
        &[
            "create",
            "--title",
            "transition matrix dependent ordinary work fixture",
            "--slug",
            "transition-matrix-dependent-work",
            "--deps",
            &format!("blocks:{decision_id}"),
            "--json",
        ],
    )?;
    let ordinary_id = create_issue(
        program,
        workspace,
        &[
            "create",
            "--title",
            "transition matrix ordinary work fixture",
            "--slug",
            "transition-matrix-ordinary-work",
            "--json",
        ],
    )?;
    let stale_id = create_issue(
        program,
        workspace,
        &[
            "create",
            "--title",
            "transition matrix stale export fixture",
            "--slug",
            "transition-matrix-stale-export",
            "--json",
        ],
    )?;

    let mut cases = Vec::with_capacity(CASE_IDS.len());
    set_fixture_state(
        program,
        workspace,
        &decision_id,
        "decision",
        &DECISION_LABELS,
    )?;
    cases.push(record_safe(
        program,
        workspace,
        "active-decision",
        "direct fixture status=decision",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        "Active owner decisions are non-ready and require both labels.",
    )?);

    let closed_before = state_json(program, workspace, &decision_id)?;
    let close = program.must_command(
        &workspace.root,
        &workspace.db,
        &[
            "close",
            &decision_id,
            "--actor",
            "thethracian",
            "--reason",
            "owner resolved decision",
            "--transition-comment",
            "Owner decision closed.",
            "--json",
        ],
    )?;
    let closed_after = state_json(program, workspace, &decision_id)?;
    require_field_equal(&closed_before, &closed_after, "created_at", "owner-close")?;
    require_labels(&closed_after, &DECISION_LABELS, "owner-close")?;
    let closed_at = closed_after
        .get("closed_at")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let close_reason = closed_after
        .get("close_reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if closed_at.is_empty() || close_reason != "owner resolved decision" {
        return Err(case_failure(
            "owner-close",
            "close provenance was incomplete",
        ));
    }
    let dependent_ready = ready_state(program, workspace, &dependent_id)?;
    if !dependent_ready {
        return Err(case_failure(
            "owner-close",
            "closing the decision did not unblock its dependent",
        ));
    }
    cases.push(CaseReport {
        case_id: "owner-close".to_owned(),
        operation: "br close with provenance".to_owned(),
        expected: ExpectedState::new("closed", &DECISION_LABELS, false, "valid").value(),
        observed: json!({
            "status": closed_after.get("status"),
            "labels": sorted_labels(&closed_after),
            "ready": false,
            "policy": "valid",
            "close_reason": close_reason,
            "actor": "thethracian",
            "dependent_ready": dependent_ready,
            "close_command_status": close.code,
        }),
        outcome: CaseOutcome::Pass,
        evidence: "Close preserved created_at, both decision labels, close provenance, and dependent readiness.".to_owned(),
    });

    let reopen = program.command(
        &workspace.root,
        Some(&workspace.db),
        &[
            "reopen",
            &decision_id,
            "--actor",
            "thethracian",
            "--reason",
            "owner reconsidered decision",
            "--json",
        ],
        false,
    )?;
    let raw_reopen = record_unsafe(
        program,
        workspace,
        "raw-reopen",
        "br reopen before restoration",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        format!(
            "br exit={:?}; actor metadata is cooperative, not owner authentication.",
            reopen.code
        ),
    )?;
    cases.push(raw_reopen);

    set_fixture_state(
        program,
        workspace,
        &decision_id,
        "decision",
        &DECISION_LABELS,
    )?;
    cases.push(record_safe(
        program,
        workspace,
        "reopen-restore",
        "direct fixture status=decision after br reopen",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        "Restoration returns the issue to the non-ready decision workflow.",
    )?);

    let claim = program.command(
        &workspace.root,
        Some(&workspace.db),
        &[
            "update",
            &decision_id,
            "--claim",
            "--actor",
            "AgentA",
            "--json",
        ],
        false,
    )?;
    cases.push(record_unsafe(
        program,
        workspace,
        "invalid-claim",
        "br update --claim",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        format!(
            "br exit={:?}; a successful claim changes status but does not authenticate AgentA.",
            claim.code
        ),
    )?);
    set_fixture_state(
        program,
        workspace,
        &decision_id,
        "decision",
        &DECISION_LABELS,
    )?;
    cases.push(record_safe(
        program,
        workspace,
        "claim-restored",
        "restore decision after claim experiment",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        "Claim experiments never become authoritative owner proof.",
    )?);

    let relabel = program.command(
        &workspace.root,
        Some(&workspace.db),
        &[
            "label",
            "remove",
            "-l",
            "decision-needed",
            &decision_id,
            "--actor",
            "AgentA",
            "--json",
        ],
        false,
    )?;
    program.must_command(&workspace.root, &workspace.db, &["sync", "--flush-only"])?;
    cases.push(record_unsafe(
        program,
        workspace,
        "invalid-relabel",
        "br label remove decision-needed",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        format!(
            "br exit={:?}; label-only drift is rejected by the cooperative validator.",
            relabel.code
        ),
    )?);
    set_fixture_state(
        program,
        workspace,
        &decision_id,
        "decision",
        &DECISION_LABELS,
    )?;
    cases.push(record_safe(
        program,
        workspace,
        "relabel-restored",
        "restore both decision labels",
        &decision_id,
        ExpectedState::new("decision", &DECISION_LABELS, false, "valid"),
        "Restoration removes the label-only drift.",
    )?);

    set_fixture_state(program, workspace, &ordinary_id, "decision", &EMPTY_LABELS)?;
    cases.push(record_safe(
        program,
        workspace,
        "ordinary-status-drift",
        "direct fixture ordinary status=decision",
        &ordinary_id,
        ExpectedState::new("decision", &EMPTY_LABELS, false, "invalid"),
        "Ordinary work cannot carry decision status without both labels.",
    )?);
    set_fixture_state(program, workspace, &ordinary_id, "open", &DECISION_LABELS)?;
    cases.push(record_safe(
        program,
        workspace,
        "ordinary-label-drift",
        "direct fixture ordinary labels=decision labels",
        &ordinary_id,
        ExpectedState::new("open", &DECISION_LABELS, true, "invalid"),
        "Label-only drift would leak ordinary work into br ready; policy rejects it.",
    )?);
    set_fixture_state(program, workspace, &ordinary_id, "open", &EMPTY_LABELS)?;
    cases.push(record_safe(
        program,
        workspace,
        "ordinary-work",
        "ordinary open fixture",
        &ordinary_id,
        ExpectedState::new("open", &EMPTY_LABELS, true, "valid"),
        "Ordinary work is ready only when it carries neither decision label.",
    )?);

    let stale_jsonl_before = jsonl_state(workspace, &stale_id)?;
    program.must_command_no_flush(
        &workspace.root,
        &workspace.db,
        &[
            "update",
            &stale_id,
            "--status",
            "in_progress",
            "--actor",
            "AgentA",
            "--json",
        ],
    )?;
    let stale_db = state_json(program, workspace, &stale_id)?;
    let stale_jsonl_after = jsonl_state(workspace, &stale_id)?;
    let stale_status = program.must_command(
        &workspace.root,
        &workspace.db,
        &["sync", "--status", "--json"],
    )?;
    let stale_status_json = parse_json("br sync --status", &stale_status.stdout)?;
    let db_status = string_field(&stale_db, "status", "stale-export")?;
    let jsonl_before_status = string_field(&stale_jsonl_before, "status", "stale-export")?;
    let jsonl_after_status = string_field(&stale_jsonl_after, "status", "stale-export")?;
    let workspace_health = stale_status_json
        .get("workspace_health")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if db_status != "in_progress" || jsonl_before_status != "open" || jsonl_after_status != "open" {
        return Err(case_failure(
            "stale-export",
            "database and JSONL did not diverge",
        ));
    }
    if workspace_health != "degraded" {
        return Err(case_failure(
            "stale-export",
            "br sync status did not report degraded workspace health",
        ));
    }
    cases.push(CaseReport {
        case_id: "stale-export".to_owned(),
        operation: "br database mutation without JSONL export".to_owned(),
        expected: ExpectedState::new("in_progress", &EMPTY_LABELS, false, "blocked").value(),
        observed: json!({
            "db_status": db_status,
            "jsonl_status": jsonl_after_status,
            "ready": false,
            "policy": "blocked",
            "workspace_health": workspace_health,
        }),
        outcome: CaseOutcome::Pass,
        evidence: "Database status differs from tracked JSONL; sync status marks the workspace degraded before export.".to_owned(),
    });
    program.must_command(&workspace.root, &workspace.db, &["sync", "--flush-only"])?;
    set_fixture_state(program, workspace, &stale_id, "open", &EMPTY_LABELS)?;
    cases.push(record_safe(
        program,
        workspace,
        "stale-restored",
        "restore synchronized ordinary fixture",
        &stale_id,
        ExpectedState::new("open", &EMPTY_LABELS, true, "valid"),
        "Stale export is repaired before readiness is consumed.",
    )?);

    if cases.len() != CASE_IDS.len() {
        return Err(case_failure(
            "matrix",
            "frozen transition case count changed",
        ));
    }
    Ok(MatrixReport {
        schema: TRANSITION_MATRIX_SCHEMA,
        cases,
        live_before_hash: String::new(),
        live_after_hash: String::new(),
        live_snapshot_unchanged: false,
        workspace_removed: false,
    })
}

fn create_issue(
    program: &BrProgram,
    workspace: &TempWorkspace,
    args: &[&str],
) -> Result<String, MatrixError> {
    let output = program.must_command(&workspace.root, &workspace.db, args)?;
    let value = parse_json(&args.join(" "), &output.stdout)?;
    value
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .as_array()
                .and_then(|items| items.first())
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .ok_or_else(|| MatrixError::InvalidJson {
            operation: args.join(" "),
            detail: "created issue has no id".to_owned(),
        })
}

fn state_json(
    program: &BrProgram,
    workspace: &TempWorkspace,
    issue_id: &str,
) -> Result<Value, MatrixError> {
    let output = program.must_command(
        &workspace.root,
        &workspace.db,
        &["show", issue_id, "--json"],
    )?;
    let value = parse_json("br show", &output.stdout)?;
    value
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .or_else(|| value.is_object().then_some(value))
        .ok_or_else(|| MatrixError::InvalidJson {
            operation: "br show".to_owned(),
            detail: "show output has no issue object".to_owned(),
        })
}

fn ready_state(
    program: &BrProgram,
    workspace: &TempWorkspace,
    issue_id: &str,
) -> Result<bool, MatrixError> {
    let output = program.must_command(&workspace.root, &workspace.db, &["ready", "--json"])?;
    let value = parse_json("br ready", &output.stdout)?;
    Ok(value
        .as_array()
        .map(|items| {
            items
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(issue_id))
        })
        .unwrap_or(false))
}

fn jsonl_state(workspace: &TempWorkspace, issue_id: &str) -> Result<Value, MatrixError> {
    let path = workspace.root.join(".beads/issues.jsonl");
    let text = fs::read_to_string(&path).map_err(|source| MatrixError::Io {
        operation: "read sandbox JSONL",
        path: path.clone(),
        source,
    })?;
    for (line_number, line) in text.lines().enumerate() {
        let value: Value =
            serde_json::from_str(line).map_err(|error| MatrixError::InvalidJson {
                operation: format!("sandbox JSONL line {}", line_number + 1),
                detail: error.to_string(),
            })?;
        if value.get("id").and_then(Value::as_str) == Some(issue_id) {
            return Ok(value);
        }
    }
    Err(MatrixError::InvalidJson {
        operation: "sandbox JSONL".to_owned(),
        detail: format!("issue {issue_id} was not found"),
    })
}

fn set_fixture_state(
    program: &BrProgram,
    workspace: &TempWorkspace,
    issue_id: &str,
    status: &str,
    labels: &[&str],
) -> Result<(), MatrixError> {
    let path = workspace.root.join(".beads/issues.jsonl");
    let source = fs::read_to_string(&path).map_err(|source| MatrixError::Io {
        operation: "read sandbox JSONL",
        path: path.clone(),
        source,
    })?;
    let mut found = false;
    let mut updated = String::new();
    for line in source.lines() {
        let mut value: Value =
            serde_json::from_str(line).map_err(|error| MatrixError::InvalidJson {
                operation: "sandbox JSONL fixture".to_owned(),
                detail: error.to_string(),
            })?;
        if value.get("id").and_then(Value::as_str) == Some(issue_id) {
            value["status"] = Value::String(status.to_owned());
            value["labels"] = Value::Array(
                labels
                    .iter()
                    .map(|label| Value::String((*label).to_owned()))
                    .collect(),
            );
            found = true;
        }
        updated.push_str(&serde_json::to_string(&value).map_err(|error| {
            MatrixError::InvalidJson {
                operation: "serialize sandbox JSONL fixture".to_owned(),
                detail: error.to_string(),
            }
        })?);
        updated.push('\n');
    }
    if !found {
        return Err(MatrixError::InvalidJson {
            operation: "sandbox JSONL fixture".to_owned(),
            detail: format!("issue {issue_id} was not found"),
        });
    }
    let next = path.with_extension("jsonl.next");
    fs::write(&next, updated).map_err(|source| MatrixError::Io {
        operation: "write sandbox JSONL fixture",
        path: next.clone(),
        source,
    })?;
    fs::rename(&next, &path).map_err(|source| MatrixError::Io {
        operation: "replace sandbox JSONL fixture",
        path: path.clone(),
        source,
    })?;
    workspace.import_rebuild(program)
}

#[derive(Debug, Clone, Copy)]
struct ExpectedState<'a> {
    status: &'a str,
    labels: &'a [&'a str],
    ready: bool,
    policy: &'a str,
}

impl<'a> ExpectedState<'a> {
    fn new(status: &'a str, labels: &'a [&'a str], ready: bool, policy: &'a str) -> Self {
        Self {
            status,
            labels,
            ready,
            policy,
        }
    }

    fn value(self) -> Value {
        json!({
            "status": self.status,
            "labels": self.labels.to_vec(),
            "ready": self.ready,
            "policy": self.policy,
        })
    }
}

fn record_safe(
    program: &BrProgram,
    workspace: &TempWorkspace,
    case_id: &str,
    operation: &str,
    issue_id: &str,
    expected_state: ExpectedState<'_>,
    evidence: &str,
) -> Result<CaseReport, MatrixError> {
    let observed = observed_state(program, workspace, issue_id)?;
    let expected = expected_state.value();
    if observed != expected {
        return Err(case_failure(
            case_id,
            &format!("expected {expected}, observed {observed}"),
        ));
    }
    Ok(CaseReport {
        case_id: bounded_text(case_id),
        operation: bounded_text(operation),
        expected,
        observed,
        outcome: CaseOutcome::Pass,
        evidence: bounded_text(evidence),
    })
}

fn record_unsafe(
    program: &BrProgram,
    workspace: &TempWorkspace,
    case_id: &str,
    operation: &str,
    issue_id: &str,
    expected_state: ExpectedState<'_>,
    evidence: String,
) -> Result<CaseReport, MatrixError> {
    let observed = observed_state(program, workspace, issue_id)?;
    let expected = expected_state.value();
    let outcome = if observed == expected {
        CaseOutcome::ToolRejected
    } else if observed.get("policy").and_then(Value::as_str) == Some("invalid") {
        CaseOutcome::UnsafeToolTransition
    } else {
        return Err(case_failure(
            case_id,
            &format!("unexpected unsafe state {observed}"),
        ));
    };
    Ok(CaseReport {
        case_id: bounded_text(case_id),
        operation: bounded_text(operation),
        expected,
        observed,
        outcome,
        evidence: bounded_text(&evidence),
    })
}

fn observed_state(
    program: &BrProgram,
    workspace: &TempWorkspace,
    issue_id: &str,
) -> Result<Value, MatrixError> {
    let state = state_json(program, workspace, issue_id)?;
    Ok(json!({
        "status": state.get("status"),
        "labels": sorted_labels(&state),
        "ready": ready_state(program, workspace, issue_id)?,
        "policy": policy_state(&state),
    }))
}

fn sorted_labels(value: &Value) -> Vec<String> {
    let mut labels = value
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    labels.sort();
    labels
}

fn policy_state(value: &Value) -> &'static str {
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let labels = sorted_labels(value);
    let has_decision_needed = labels.iter().any(|label| label == DECISION_LABELS[0]);
    let has_human_input = labels.iter().any(|label| label == DECISION_LABELS[1]);
    let decision_labels = has_decision_needed || has_human_input;
    let valid = match status {
        "decision" => has_decision_needed && has_human_input,
        "closed" => (has_decision_needed && has_human_input) || !decision_labels,
        _ => !decision_labels,
    };
    if valid { "valid" } else { "invalid" }
}

fn require_field_equal(
    before: &Value,
    after: &Value,
    field: &str,
    case_id: &str,
) -> Result<(), MatrixError> {
    if before.get(field) != after.get(field) {
        return Err(case_failure(
            case_id,
            &format!("{field} changed during transition"),
        ));
    }
    Ok(())
}

fn require_labels(value: &Value, labels: &[&str], case_id: &str) -> Result<(), MatrixError> {
    let mut expected = labels
        .iter()
        .map(|label| (*label).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    if sorted_labels(value) != expected {
        return Err(case_failure(case_id, "decision labels were not preserved"));
    }
    Ok(())
}

fn string_field<'a>(value: &'a Value, field: &str, case_id: &str) -> Result<&'a str, MatrixError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| case_failure(case_id, &format!("missing string field {field}")))
}

fn parse_json(operation: &str, output: &[u8]) -> Result<Value, MatrixError> {
    serde_json::from_slice(output).map_err(|error| MatrixError::InvalidJson {
        operation: operation.to_owned(),
        detail: bounded_text(&error.to_string()),
    })
}

fn case_failure(case_id: &str, detail: &str) -> MatrixError {
    MatrixError::CaseFailed {
        case_id: bounded_text(case_id),
        detail: bounded_text(detail),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveSnapshot {
    files: BTreeMap<String, FileDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileDigest {
    len: u64,
    hash: u64,
}

impl LiveSnapshot {
    fn capture(root: &Path) -> Result<Self, MatrixError> {
        if !root.is_dir() {
            return Err(MatrixError::LiveBeadsMissing {
                path: root.to_path_buf(),
            });
        }
        let mut files = BTreeMap::new();
        let export = root.join("issues.jsonl");
        let bytes = fs::read(&export).map_err(|source| MatrixError::Io {
            operation: "read live Beads export",
            path: export,
            source,
        })?;
        files.insert(
            "issues.jsonl".to_owned(),
            FileDigest {
                len: bytes.len() as u64,
                hash: fnv1a64(&bytes),
            },
        );
        Ok(Self { files })
    }

    fn digest(&self) -> String {
        let mut bytes = Vec::new();
        for (path, digest) in &self.files {
            bytes.extend_from_slice(path.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(digest.len.to_string().as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(format!("{:016x}", digest.hash).as_bytes());
            bytes.push(b'\n');
        }
        format!("fnv1a64:{:016x}", fnv1a64(&bytes))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
fn copy_file(source: &Path, destination: &Path) -> Result<(), MatrixError> {
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(|source_error| MatrixError::Io {
            operation: "copy Beads fixture",
            path: destination.to_path_buf(),
            source: source_error,
        })
}

fn bounded_text(value: &str) -> String {
    if value.len() <= MAX_DIAGNOSTIC_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn bounded_bytes(value: &[u8]) -> String {
    bounded_text(&String::from_utf8_lossy(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn process_output(stdout: Vec<u8>, stderr: Vec<u8>) -> Output {
        use std::os::unix::process::ExitStatusExt;

        Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout,
            stderr,
        }
    }

    #[cfg(windows)]
    fn process_output(stdout: Vec<u8>, stderr: Vec<u8>) -> Output {
        use std::os::windows::process::ExitStatusExt;

        Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout,
            stderr,
        }
    }

    #[test]
    fn valid_json_above_diagnostic_bound_is_not_truncated_before_parse() {
        let payload = serde_json::to_vec(&json!({
            "id": "ready",
            "padding": "x".repeat(4096),
        }))
        .expect("serialize oversized JSON fixture");
        assert!(payload.len() > 4096);

        let output = BrOutput::from_output(process_output(payload, Vec::new()));
        assert!(output.stdout.len() > 4096);
        let parsed = parse_json("oversized valid JSON", &output.stdout)
            .expect("complete admitted JSON must parse");
        assert_eq!(parsed["id"], "ready");
    }

    #[test]
    fn output_above_machine_admission_bound_fails_before_parse() {
        let output = BrOutput::from_parts(Some(0), vec![b'x'; MAX_OUTPUT_BYTES + 1], Vec::new());
        let error = admit_output("br ready --json".to_owned(), output)
            .expect_err("output above the machine bound must fail closed");
        match error {
            MatrixError::BrFailed {
                operation,
                stdout,
                stderr,
                ..
            } => {
                assert_eq!(operation, "br ready --json");
                assert_eq!(stdout, "output exceeded the transition matrix bound");
                assert!(stderr.is_empty());
            }
            other => panic!("unexpected oversized-output error: {other:?}"),
        }
    }

    #[test]
    fn invalid_admitted_json_returns_bounded_parse_diagnostic() {
        let output = BrOutput::from_parts(
            Some(0),
            format!(
                "{{\"padding\":\"{}",
                "x".repeat(MAX_DIAGNOSTIC_BYTES + 1024)
            )
            .into_bytes(),
            Vec::new(),
        );
        let output = admit_output("br ready --json".to_owned(), output)
            .expect("invalid fixture remains below the machine bound");
        let error = parse_json("br ready --json", &output.stdout)
            .expect_err("invalid JSON must not be accepted");
        assert!(matches!(error, MatrixError::InvalidJson { .. }));
        assert!(error.to_string().len() <= MAX_DIAGNOSTIC_BYTES + 128);
    }

    #[test]
    fn child_failure_renders_bounded_stdout_and_stderr_diagnostics() {
        let output = BrOutput::from_parts(
            Some(7),
            format!("stdout:{}", "s".repeat(MAX_DIAGNOSTIC_BYTES + 1024)).into_bytes(),
            format!("stderr:{}", "e".repeat(MAX_DIAGNOSTIC_BYTES + 1024)).into_bytes(),
        );
        let error = output.into_failure("br ready --json".to_owned());
        match &error {
            MatrixError::BrFailed { stdout, stderr, .. } => {
                assert!(stdout.len() <= MAX_DIAGNOSTIC_BYTES);
                assert!(stderr.len() <= MAX_DIAGNOSTIC_BYTES);
                assert!(!stdout.contains('…'));
                assert!(!stderr.contains('…'));
            }
            other => panic!("unexpected child-failure error: {other:?}"),
        }
        assert!(error.to_string().len() <= (MAX_DIAGNOSTIC_BYTES * 2) + 128);
    }

    #[test]
    fn live_diagnostic_hash_ignores_concurrent_tracker_metadata() {
        use std::sync::{Arc, Barrier};

        let root = std::env::temp_dir().join(format!(
            "omnirepo-transition-matrix-snapshot-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create snapshot fixture");
        fs::write(root.join("issues.jsonl"), b"canonical export\n").expect("write export");
        let barrier = Arc::new(Barrier::new(2));
        let writer_barrier = Arc::clone(&barrier);
        let writer_root = root.clone();
        let writer = thread::spawn(move || {
            writer_barrier.wait();
            fs::write(
                writer_root.join(".br_history"),
                b"legitimate concurrent write\n",
            )
            .expect("write volatile tracker metadata");
        });

        let before = LiveSnapshot::capture(&root).expect("capture stable export");
        barrier.wait();
        writer.join().expect("metadata writer must not panic");
        let after = LiveSnapshot::capture(&root).expect("capture stable export again");
        assert_eq!(before, after);
        fs::remove_dir_all(root).expect("remove snapshot fixture");
    }

    #[test]
    fn live_export_drift_is_diagnostic_only() {
        // This is a live-beads test: it needs the owner-machine `br` CLI on
        // PATH.  CI cannot install it, so the test skips with a visible
        // note there and runs for real on the owner machine.
        if !crate::br_adapter::BrAdapterConfig::is_br_on_path() {
            eprintln!(
                "authority-capability: skipped-live-br (the owner-machine br CLI is not installed)"
            );
            return;
        }
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("developer-tool manifest is nested below repository root");
        let live_source = repository_root.join(".beads");
        let fixture_root = unique_temp_directory().expect("create live fixture root");
        let fixture_beads = fixture_root.join(".beads");
        fs::create_dir(&fixture_beads).expect("create live fixture Beads directory");
        for name in [
            "issues.jsonl",
            "policy.yaml",
            "config.yaml",
            "metadata.json",
        ] {
            copy_file(&live_source.join(name), &fixture_beads.join(name))
                .expect("copy canonical fixture input");
        }
        let hook = |live_beads: &Path| {
            let export = live_beads.join("issues.jsonl");
            let mut changed = fs::read(&export).expect("read frozen fixture export");
            changed.push(b'\n');
            fs::write(export, changed).expect("write fixture export drift");
        };
        let report = run_with_br_path_and_timeout_hook(
            &fixture_root,
            PathBuf::from("br"),
            DEFAULT_COMMAND_TIMEOUT,
            Some(&hook),
        )
        .expect("fixture matrix run should pass");
        assert!(!report.live_snapshot_unchanged);
        assert_eq!(report.cases.len(), CASE_IDS.len());
        fs::remove_dir_all(fixture_root).expect("remove live fixture root");
    }
}
