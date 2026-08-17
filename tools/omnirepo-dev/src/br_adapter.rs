//! A bounded, read-only process boundary for the Beads `br` command.
//!
//! The checked planner is allowed to consume only the two canonical tracker
//! snapshots exposed by `br`: `ready --json` and `scheduler --json`.  This
//! module owns the process boundary.  It does not parse tracker data, invoke
//! a shell, or call Viewer (`bv`).

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The maximum number of bytes captured from either child output stream.
///
/// The limit is deliberately the same order as the repository lifecycle
/// evidence limit.  A command that exceeds it is terminated and reported as
/// an adapter failure; output is never silently truncated into a valid plan.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// The default bound for one read-only `br` operation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// A stable maximum length for diagnostic text included in typed errors.
pub const MAX_DIAGNOSTIC_TEXT: usize = 256;

const READY_ARGS: &[&str] = &["--no-auto-import", "--no-auto-flush", "ready", "--json"];
const SCHEDULER_ARGS: &[&str] = &["--no-auto-import", "--no-auto-flush", "scheduler", "--json"];

/// Which canonical read-only tracker source was requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Ready,
    Scheduler,
}

impl SourceKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Scheduler => "scheduler",
        }
    }

    const fn args(self) -> &'static [&'static str] {
        match self {
            Self::Ready => READY_ARGS,
            Self::Scheduler => SCHEDULER_ARGS,
        }
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// One successful canonical `br` capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceOutput {
    pub source: SourceKind,
    pub stdout: String,
    pub stderr: String,
}

/// Captured diagnostics attached to a failed child process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessDiagnostics {
    pub stdout: String,
    pub stderr: String,
}

impl ProcessDiagnostics {
    fn new(stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            stdout: bounded_text(&String::from_utf8_lossy(&stdout)),
            stderr: bounded_text(&String::from_utf8_lossy(&stderr)),
        }
    }
}

/// Stable errors from the `br` process boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrAdapterError {
    InvalidRepositoryRoot {
        path: PathBuf,
        reason: String,
    },
    MissingExecutable {
        executable: PathBuf,
    },
    IncompatibleExecutable {
        executable: PathBuf,
        reason: String,
    },
    InvalidTimeout {
        timeout: Duration,
        reason: String,
    },
    Spawn {
        source: SourceKind,
        command: String,
        reason: String,
    },
    NonZero {
        source: SourceKind,
        status: String,
        diagnostics: ProcessDiagnostics,
    },
    Timeout {
        source: SourceKind,
        timeout: Duration,
        diagnostics: ProcessDiagnostics,
    },
    OutputTooLarge {
        source: SourceKind,
        stream: &'static str,
        limit: usize,
        diagnostics: ProcessDiagnostics,
    },
    InvalidUtf8 {
        source: SourceKind,
        stream: &'static str,
        diagnostics: ProcessDiagnostics,
    },
    Read {
        source: SourceKind,
        stream: &'static str,
        reason: String,
        diagnostics: ProcessDiagnostics,
    },
    Wait {
        source: SourceKind,
        reason: String,
        diagnostics: ProcessDiagnostics,
    },
}

impl BrAdapterError {
    /// The source command that produced this error, when one was started.
    pub const fn source_kind(&self) -> Option<SourceKind> {
        match self {
            Self::InvalidRepositoryRoot { .. }
            | Self::MissingExecutable { .. }
            | Self::IncompatibleExecutable { .. }
            | Self::InvalidTimeout { .. } => None,
            Self::Spawn { source, .. }
            | Self::NonZero { source, .. }
            | Self::Timeout { source, .. }
            | Self::OutputTooLarge { source, .. }
            | Self::InvalidUtf8 { source, .. }
            | Self::Read { source, .. }
            | Self::Wait { source, .. } => Some(*source),
        }
    }

    /// Return bounded child diagnostics if a process was started.
    pub const fn diagnostics(&self) -> Option<&ProcessDiagnostics> {
        match self {
            Self::NonZero { diagnostics, .. }
            | Self::Timeout { diagnostics, .. }
            | Self::OutputTooLarge { diagnostics, .. }
            | Self::InvalidUtf8 { diagnostics, .. }
            | Self::Read { diagnostics, .. }
            | Self::Wait { diagnostics, .. } => Some(diagnostics),
            Self::InvalidRepositoryRoot { .. }
            | Self::MissingExecutable { .. }
            | Self::IncompatibleExecutable { .. }
            | Self::InvalidTimeout { .. }
            | Self::Spawn { .. } => None,
        }
    }

    /// A stable machine-oriented reason for this boundary failure.
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidRepositoryRoot { .. } => "invalid-repository-root",
            Self::MissingExecutable { .. } => "required-command-missing",
            Self::IncompatibleExecutable { .. } => "incompatible-command",
            Self::InvalidTimeout { .. } => "invalid-timeout",
            Self::Spawn { .. } => "canonical-source-command-failed",
            Self::NonZero { .. } => "canonical-source-command-failed",
            Self::Timeout { .. } => "canonical-source-timeout",
            Self::OutputTooLarge { .. } => "canonical-source-output-too-large",
            Self::InvalidUtf8 { .. } => "canonical-source-invalid-utf8",
            Self::Read { .. } => "canonical-source-read-failed",
            Self::Wait { .. } => "canonical-source-wait-failed",
        }
    }
}

impl fmt::Display for BrAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepositoryRoot { path, reason } => {
                write!(
                    formatter,
                    "invalid repository root {}: {reason}",
                    path.display()
                )
            }
            Self::MissingExecutable { executable } => {
                write!(
                    formatter,
                    "required br executable is missing: {}",
                    executable.display()
                )
            }
            Self::IncompatibleExecutable { executable, reason } => write!(
                formatter,
                "br executable is incompatible ({}): {reason}",
                executable.display()
            ),
            Self::InvalidTimeout { timeout, reason } => write!(
                formatter,
                "br timeout {:?} cannot be represented: {reason}",
                timeout
            ),
            Self::Spawn {
                source,
                command,
                reason,
            } => write!(formatter, "cannot start br {source} ({command}): {reason}"),
            Self::NonZero {
                source,
                status,
                diagnostics,
            } => write!(
                formatter,
                "br {source} exited {status}: {}",
                diagnostic_suffix(diagnostics)
            ),
            Self::Timeout {
                source,
                timeout,
                diagnostics,
            } => write!(
                formatter,
                "br {source} exceeded {:?}: {}",
                timeout,
                diagnostic_suffix(diagnostics)
            ),
            Self::OutputTooLarge {
                source,
                stream,
                limit,
                diagnostics,
            } => write!(
                formatter,
                "br {source} {stream} exceeded {limit} bytes: {}",
                diagnostic_suffix(diagnostics)
            ),
            Self::InvalidUtf8 {
                source,
                stream,
                diagnostics,
            } => write!(
                formatter,
                "br {source} {stream} was not UTF-8: {}",
                diagnostic_suffix(diagnostics)
            ),
            Self::Read {
                source,
                stream,
                reason,
                diagnostics,
            } => write!(
                formatter,
                "br {source} {stream} read failed ({reason}): {}",
                diagnostic_suffix(diagnostics)
            ),
            Self::Wait {
                source,
                reason,
                diagnostics,
            } => write!(
                formatter,
                "cannot reap br {source}: {reason}: {}",
                diagnostic_suffix(diagnostics)
            ),
        }
    }
}

impl std::error::Error for BrAdapterError {}

/// Configuration for one frozen, read-only adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrAdapterConfig {
    pub repository_root: PathBuf,
    pub executable: PathBuf,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl BrAdapterConfig {
    /// Whether the owner-machine `br` CLI is discoverable on PATH.  CI
    /// cannot install it; live-beads tests use this to skip with a visible
    /// note instead of failing on a missing tracker tool.
    pub fn is_br_on_path() -> bool {
        find_on_path(OsStr::new("br")).ok().flatten().is_some()
    }

    /// Discover `br` from the caller's PATH and freeze the resulting identity.
    pub fn discover(repository_root: impl AsRef<Path>) -> Result<Self, BrAdapterError> {
        let executable =
            find_on_path(OsStr::new("br"))?.ok_or_else(|| BrAdapterError::MissingExecutable {
                executable: PathBuf::from("br"),
            })?;
        Self::with_executable(repository_root, executable)
    }

    /// Freeze an explicitly selected executable for tests and controlled hosts.
    pub fn with_executable(
        repository_root: impl AsRef<Path>,
        executable: impl AsRef<Path>,
    ) -> Result<Self, BrAdapterError> {
        let repository_root = canonical_repository_root(repository_root.as_ref())?;
        let executable = executable.as_ref().to_owned();
        let executable = if executable.is_absolute() {
            executable
        } else {
            find_on_path(executable.as_os_str())?.ok_or_else(|| {
                BrAdapterError::MissingExecutable {
                    executable: executable.clone(),
                }
            })?
        };
        let executable = executable.canonicalize().map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                BrAdapterError::MissingExecutable {
                    executable: executable.clone(),
                }
            } else {
                BrAdapterError::IncompatibleExecutable {
                    executable: executable.clone(),
                    reason: error.to_string(),
                }
            }
        })?;
        let metadata =
            fs::metadata(&executable).map_err(|error| BrAdapterError::IncompatibleExecutable {
                executable: executable.clone(),
                reason: error.to_string(),
            })?;
        if !metadata.is_file() {
            return Err(BrAdapterError::IncompatibleExecutable {
                executable,
                reason: "path is not a regular file".to_owned(),
            });
        }
        if !is_executable_file(&executable) {
            return Err(BrAdapterError::IncompatibleExecutable {
                executable,
                reason: "file is not executable".to_owned(),
            });
        }

        Ok(Self {
            repository_root,
            executable,
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        })
    }

    /// Set the operation bound.  An unrepresentable deadline is rejected by
    /// [`BrAdapter::ready`] or [`BrAdapter::scheduler`] as
    /// [`BrAdapterError::InvalidTimeout`].
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub const fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }
}

/// A read-only adapter using one frozen repository and executable identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrAdapter {
    config: BrAdapterConfig,
}

impl BrAdapter {
    pub fn discover(repository_root: impl AsRef<Path>) -> Result<Self, BrAdapterError> {
        Ok(Self {
            config: BrAdapterConfig::discover(repository_root)?,
        })
    }

    pub fn new(config: BrAdapterConfig) -> Self {
        Self { config }
    }

    pub const fn config(&self) -> &BrAdapterConfig {
        &self.config
    }

    /// Read the canonical `br ready --json` snapshot.
    pub fn ready(&self) -> Result<SourceOutput, BrAdapterError> {
        self.run(SourceKind::Ready)
    }

    /// Read the canonical `br scheduler --json` snapshot.
    pub fn scheduler(&self) -> Result<SourceOutput, BrAdapterError> {
        self.run(SourceKind::Scheduler)
    }

    fn run(&self, source: SourceKind) -> Result<SourceOutput, BrAdapterError> {
        let args = source.args();
        let command = command_display(&self.config.executable, args);
        let deadline = Instant::now()
            .checked_add(self.config.timeout)
            .ok_or_else(|| BrAdapterError::InvalidTimeout {
                timeout: self.config.timeout,
                reason: "deadline would overflow the platform clock".to_owned(),
            })?;
        let mut child_command = Command::new(&self.config.executable);
        child_command
            .args(args)
            .current_dir(&self.config.repository_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group(&mut child_command);
        install_sanitized_environment(
            &mut child_command,
            &self.config.repository_root,
            &self.config.executable,
        );

        let mut child = child_command
            .spawn()
            .map_err(|error| BrAdapterError::Spawn {
                source,
                command: bounded_text(&command),
                reason: error.to_string(),
            })?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_process_tree(&child);
                let _ = child.wait();
                return Err(BrAdapterError::Wait {
                    source,
                    reason: "stdout pipe was not created".to_owned(),
                    diagnostics: ProcessDiagnostics::new(Vec::new(), Vec::new()),
                });
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_process_tree(&child);
                let _ = child.wait();
                return Err(BrAdapterError::Wait {
                    source,
                    reason: "stderr pipe was not created".to_owned(),
                    diagnostics: ProcessDiagnostics::new(Vec::new(), Vec::new()),
                });
            }
        };
        let stdout_reader = spawn_reader(stdout, self.config.max_output_bytes, "stdout");
        let stderr_reader = spawn_reader(stderr, self.config.max_output_bytes, "stderr");

        let mut status = None;
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(exit_status)) => {
                    status = Some(exit_status);
                    // A child can leave a descendant holding a pipe open after
                    // it exits.  The process group is ours, so clean up the
                    // descendants before joining the readers.
                    terminate_process_tree(&child);
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    terminate_process_tree(&child);
                    let _ = child.wait();
                    let diagnostics = join_diagnostics(stdout_reader, stderr_reader);
                    return Err(BrAdapterError::Wait {
                        source,
                        reason: error.to_string(),
                        diagnostics,
                    });
                }
            }

            if stdout_reader.overflowed()
                || stderr_reader.overflowed()
                || stdout_reader.failed()
                || stderr_reader.failed()
            {
                terminate_process_tree(&child);
                break;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                terminate_process_tree(&child);
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }

        if status.is_none() {
            status = child.wait().ok();
        }
        let stdout_result = stdout_reader.join();
        let stderr_result = stderr_reader.join();
        let ReaderResult {
            bytes: stdout_bytes,
            overflowed: stdout_overflowed,
            error: stdout_error,
        } = stdout_result;
        let ReaderResult {
            bytes: stderr_bytes,
            overflowed: stderr_overflowed,
            error: stderr_error,
        } = stderr_result;
        let diagnostics = ProcessDiagnostics::new(stdout_bytes.clone(), stderr_bytes.clone());

        if let Some(failure) = stdout_error {
            return Err(BrAdapterError::Read {
                source,
                stream: failure.stream,
                reason: failure.reason,
                diagnostics,
            });
        }
        if stdout_overflowed {
            return Err(BrAdapterError::OutputTooLarge {
                source,
                stream: "stdout",
                limit: self.config.max_output_bytes,
                diagnostics,
            });
        }
        if stderr_overflowed {
            return Err(BrAdapterError::OutputTooLarge {
                source,
                stream: "stderr",
                limit: self.config.max_output_bytes,
                diagnostics,
            });
        }
        if let Some(failure) = stderr_error {
            return Err(BrAdapterError::Read {
                source,
                stream: failure.stream,
                reason: failure.reason,
                diagnostics,
            });
        }
        if timed_out {
            return Err(BrAdapterError::Timeout {
                source,
                timeout: self.config.timeout,
                diagnostics,
            });
        }
        let stdout = String::from_utf8(stdout_bytes).map_err(|_| BrAdapterError::InvalidUtf8 {
            source,
            stream: "stdout",
            diagnostics: diagnostics.clone(),
        })?;
        let stderr = String::from_utf8(stderr_bytes).map_err(|_| BrAdapterError::InvalidUtf8 {
            source,
            stream: "stderr",
            diagnostics: diagnostics.clone(),
        })?;

        let Some(status) = status else {
            return Err(BrAdapterError::Wait {
                source,
                reason: "child status was unavailable after wait".to_owned(),
                diagnostics,
            });
        };
        if !status.success() {
            return Err(BrAdapterError::NonZero {
                source,
                status: status_string(status),
                diagnostics,
            });
        }
        Ok(SourceOutput {
            source,
            stdout,
            stderr,
        })
    }
}

fn canonical_repository_root(path: &Path) -> Result<PathBuf, BrAdapterError> {
    let resolved = path
        .canonicalize()
        .map_err(|error| BrAdapterError::InvalidRepositoryRoot {
            path: path.to_owned(),
            reason: error.to_string(),
        })?;
    if !resolved.is_dir() {
        return Err(BrAdapterError::InvalidRepositoryRoot {
            path: path.to_owned(),
            reason: "path is not a directory".to_owned(),
        });
    }
    Ok(resolved)
}

fn find_on_path(program: &OsStr) -> Result<Option<PathBuf>, BrAdapterError> {
    let Some(path) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if is_executable_file(&candidate) {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn command_display(executable: &Path, args: &[&str]) -> String {
    let mut command = executable.display().to_string();
    for argument in args {
        command.push(' ');
        command.push_str(argument);
    }
    command
}

fn install_sanitized_environment(command: &mut Command, repository_root: &Path, executable: &Path) {
    let llvm_profile_file = std::env::var_os("LLVM_PROFILE_FILE");
    command.env_clear();
    let path = executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    command.env("PATH", path);
    command.env("HOME", repository_root);
    command.env("TMPDIR", controlled_temp_dir());
    command.env("NO_COLOR", "1");
    if let Some(profile) = llvm_profile_file {
        command.env("LLVM_PROFILE_FILE", profile);
    }
}

fn controlled_temp_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"C:\Windows\Temp")
    }
    #[cfg(not(any(unix, windows)))]
    {
        PathBuf::from(".")
    }
}

#[derive(Debug)]
struct ReaderResult {
    bytes: Vec<u8>,
    overflowed: bool,
    error: Option<ReaderFailure>,
}

#[derive(Debug)]
struct ReaderFailure {
    stream: &'static str,
    reason: String,
}

struct ReaderHandle {
    overflowed: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    join: Option<JoinHandle<ReaderResult>>,
}

impl ReaderHandle {
    fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    fn join(mut self) -> ReaderResult {
        self.join
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or(ReaderResult {
                bytes: Vec::new(),
                overflowed: self.overflowed(),
                error: self.failed().then(|| ReaderFailure {
                    stream: "unknown",
                    reason: "reader thread terminated unexpectedly".to_owned(),
                }),
            })
    }
}

fn spawn_reader<R>(mut reader: R, limit: usize, stream: &'static str) -> ReaderHandle
where
    R: Read + Send + 'static,
{
    let overflowed = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&overflowed);
    let failed = Arc::new(AtomicBool::new(false));
    let failed_marker = Arc::clone(&failed);
    let join = thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(8192));
        let mut chunk = [0_u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    return ReaderResult {
                        bytes,
                        overflowed: marker.load(Ordering::Acquire),
                        error: None,
                    };
                }
                Ok(read) => {
                    if read > chunk.len() {
                        let reason = "reader returned more bytes than requested";
                        failed_marker.store(true, Ordering::Release);
                        return ReaderResult {
                            bytes,
                            overflowed: false,
                            error: Some(ReaderFailure {
                                stream,
                                reason: reason.to_owned(),
                            }),
                        };
                    }
                    let remaining = limit.saturating_sub(bytes.len());
                    if read > remaining {
                        bytes.extend_from_slice(&chunk[..remaining]);
                        marker.store(true, Ordering::Release);
                        return ReaderResult {
                            bytes,
                            overflowed: true,
                            error: None,
                        };
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    failed_marker.store(true, Ordering::Release);
                    return ReaderResult {
                        bytes,
                        overflowed: marker.load(Ordering::Acquire),
                        error: Some(ReaderFailure {
                            stream,
                            reason: bounded_text(&error.to_string()),
                        }),
                    };
                }
            }
        }
    });
    ReaderHandle {
        overflowed,
        failed,
        join: Some(join),
    }
}

fn join_diagnostics(stdout: ReaderHandle, stderr: ReaderHandle) -> ProcessDiagnostics {
    ProcessDiagnostics::new(stdout.join().bytes, stderr.join().bytes)
}

fn status_string(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal:{signal}");
        }
    }
    status
        .code()
        .map(|code| format!("code:{code}"))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn bounded_text(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= MAX_DIAGNOSTIC_TEXT {
        return collapsed;
    }
    let mut output = String::new();
    for character in collapsed.chars() {
        if output.len() + character.len_utf8() + '…'.len_utf8() > MAX_DIAGNOSTIC_TEXT {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}

fn diagnostic_suffix(diagnostics: &ProcessDiagnostics) -> String {
    let mut values = Vec::new();
    if !diagnostics.stderr.is_empty() {
        values.push(format!("stderr={}", diagnostics.stderr));
    }
    if !diagnostics.stdout.is_empty() {
        values.push(format!("stdout={}", diagnostics.stdout));
    }
    if values.is_empty() {
        "no diagnostics".to_owned()
    } else {
        values.join(" ")
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: `pre_exec` runs between fork and exec.  `setpgid(0, 0)` only
    // changes the child process's own process group, which lets the parent
    // terminate descendants without touching the caller's group.
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
fn terminate_process_tree(child: &Child) {
    let pid = child.id() as i32;
    // SAFETY: the process group was created by `configure_process_group` for
    // this child.  The direct-child fallback handles a failed group lookup.
    unsafe {
        let _ = kill(-pid, SIGKILL);
        let _ = kill(pid, SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_process_tree(child: &Child) {
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(child: &Child) {
    let _ = child.kill();
}

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn setpgid(pid: i32, pgid: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use super::{ReaderHandle, bounded_text, join_diagnostics, spawn_reader};

    #[test]
    fn diagnostics_collapse_and_bound_text() {
        let value = bounded_text(&format!("a  b\n{}", "x".repeat(400)));
        assert!(value.len() <= super::MAX_DIAGNOSTIC_TEXT);
        assert!(value.ends_with('…'));
        assert!(!value.contains("  "));
    }

    struct ErrorReader;

    impl Read for ErrorReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other(format!(
                "synthetic reader failure {}",
                "x".repeat(512)
            )))
        }
    }

    #[test]
    fn reader_preserves_bounded_evidence_and_typed_io_errors() {
        let handle = spawn_reader(ErrorReader, 64, "stdout");
        let result = handle.join();
        assert!(result.bytes.is_empty());
        let error = result.error.expect("reader failure is retained");
        assert_eq!(error.stream, "stdout");
        assert!(error.reason.starts_with("synthetic reader failure"));
        assert!(error.reason.len() <= super::MAX_DIAGNOSTIC_TEXT);
    }

    struct InterruptedThenData {
        interrupted: bool,
        done: bool,
    }

    impl Read for InterruptedThenData {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            if self.done {
                return Ok(0);
            }
            self.done = true;
            buffer[..3].copy_from_slice(b"out");
            Ok(3)
        }
    }

    #[test]
    fn reader_retries_interrupted_reads_before_returning_data() {
        let handle = spawn_reader(
            InterruptedThenData {
                interrupted: false,
                done: false,
            },
            64,
            "stderr",
        );
        let result = handle.join();
        assert_eq!(result.bytes, b"out");
        assert!(result.error.is_none());
    }

    struct PrefixThenError {
        emitted: bool,
    }

    impl Read for PrefixThenError {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.emitted {
                self.emitted = true;
                buffer[..6].copy_from_slice(b"prefix");
                return Ok(6);
            }
            Err(io::Error::other("reader stopped after prefix"))
        }
    }

    #[test]
    fn reader_failure_retains_the_bounded_prefix() {
        let handle = spawn_reader(PrefixThenError { emitted: false }, 64, "stdout");
        let result = handle.join();
        assert_eq!(result.bytes, b"prefix");
        assert_eq!(
            result.error.expect("reader failure is retained").reason,
            "reader stopped after prefix"
        );
    }

    #[test]
    fn reader_marks_overflow_and_keeps_only_the_admitted_prefix() {
        let handle = spawn_reader(Cursor::new(b"abcdef".to_vec()), 3, "stdout");
        let result = handle.join();
        assert_eq!(result.bytes, b"abc");
        assert!(result.overflowed);
    }

    #[test]
    fn reader_join_fallback_and_diagnostic_join_are_deterministic() {
        let marker = Arc::new(AtomicBool::new(false));
        let fallback = ReaderHandle {
            overflowed: Arc::clone(&marker),
            failed: Arc::clone(&marker),
            join: None,
        };
        assert!(fallback.join().bytes.is_empty());

        let stdout = spawn_reader(Cursor::new(b"out".to_vec()), 64, "stdout");
        let stderr = spawn_reader(Cursor::new(b"err".to_vec()), 64, "stderr");
        let diagnostics = join_diagnostics(stdout, stderr);
        assert_eq!(diagnostics.stdout, "out");
        assert_eq!(diagnostics.stderr, "err");
    }
}
