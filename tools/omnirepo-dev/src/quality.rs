//! Failure-preserving execution of the repository quality manifest.
//!
//! This module owns execution only. It does not interpret shell syntax,
//! select or install toolchains, or infer commands. The manifest's ordered
//! argument vectors and declared toolchain values remain authoritative.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

pub const REPORT_SCHEMA: &str = "omnirepo.quality-report.v1";
const DEFAULT_GATE_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;
const OUTPUT_TRUNCATION_MARKER: &str = "\n[output truncated: capture limit exceeded]\n";

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    version: u64,
    gates: Vec<ManifestGate>,
    profiles: Vec<ManifestProfile>,
}

#[derive(Debug, Deserialize)]
struct ManifestGate {
    id: String,
    kind: String,
    toolchain: String,
    working_directory: String,
    argv: Vec<String>,
    failure_identity: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    output_limit_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ManifestProfile {
    name: String,
    kind: String,
    gates: Vec<String>,
}

/// Inputs for one quality-manifest run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerOptions {
    pub manifest_path: PathBuf,
    pub repo_root: PathBuf,
    pub profile: Option<String>,
}

impl RunnerOptions {
    pub fn new(manifest_path: impl Into<PathBuf>, repo_root: impl Into<PathBuf>) -> Self {
        Self {
            manifest_path: manifest_path.into(),
            repo_root: repo_root.into(),
            profile: None,
        }
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }
}

/// The stable machine-readable result of every attempted gate.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct QualityReport {
    pub schema: &'static str,
    pub profile: String,
    pub success: bool,
    pub exit_code: i32,
    pub gates: Vec<GateResult>,
}

impl QualityReport {
    pub fn json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// One ordered gate result. Gate output is retained in full.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateResult {
    pub id: String,
    pub failure_identity: String,
    pub toolchain: String,
    pub working_directory: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<GateFailure>,
}

/// A process-boundary failure that cannot be represented by an exit status.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateFailure {
    Spawn { message: String },
    Timeout,
    OutputOverflow,
    Capture { message: String },
}

/// Errors that prevent a report from being produced.
#[derive(Debug)]
pub enum RunnerError {
    ReadManifest {
        path: PathBuf,
        source: io::Error,
    },
    ParseManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidManifest {
        path: PathBuf,
        reason: String,
    },
    UnknownProfile {
        path: PathBuf,
        profile: String,
    },
    InvalidRepositoryRoot {
        path: PathBuf,
        source: io::Error,
    },
    InvalidWorkingDirectory {
        gate_id: String,
        path: PathBuf,
        reason: String,
    },
    InvalidGateLimits {
        gate_id: String,
        reason: String,
    },
    SerializeReport(serde_json::Error),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadManifest { path, source } => {
                write!(
                    formatter,
                    "cannot read quality manifest {}: {source}",
                    path.display()
                )
            }
            Self::ParseManifest { path, source } => {
                write!(
                    formatter,
                    "cannot parse quality manifest {}: {source}",
                    path.display()
                )
            }
            Self::InvalidManifest { path, reason } => {
                write!(
                    formatter,
                    "invalid quality manifest {}: {reason}",
                    path.display()
                )
            }
            Self::UnknownProfile { path, profile } => {
                write!(
                    formatter,
                    "unknown quality profile {profile:?} in {}",
                    path.display()
                )
            }
            Self::InvalidRepositoryRoot { path, source } => {
                write!(
                    formatter,
                    "cannot resolve repository root {}: {source}",
                    path.display()
                )
            }
            Self::InvalidWorkingDirectory {
                gate_id,
                path,
                reason,
            } => write!(
                formatter,
                "invalid working directory for gate {gate_id} ({}): {reason}",
                path.display()
            ),
            Self::InvalidGateLimits { gate_id, reason } => {
                write!(
                    formatter,
                    "invalid process limits for gate {gate_id}: {reason}"
                )
            }
            Self::SerializeReport(source) => {
                write!(formatter, "cannot serialize quality report: {source}")
            }
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReadManifest { source, .. } | Self::InvalidRepositoryRoot { source, .. } => {
                Some(source)
            }
            Self::ParseManifest { source, .. } | Self::SerializeReport(source) => Some(source),
            Self::InvalidManifest { .. }
            | Self::UnknownProfile { .. }
            | Self::InvalidWorkingDirectory { .. }
            | Self::InvalidGateLimits { .. } => None,
        }
    }
}

/// Run every gate in manifest order and retain all outcomes.
///
/// The child environment is inherited from this process. The declared
/// toolchain is copied into the report and is never used to rewrite argv.
pub fn run(options: &RunnerOptions) -> Result<QualityReport, RunnerError> {
    let source =
        fs::read_to_string(&options.manifest_path).map_err(|source| RunnerError::ReadManifest {
            path: options.manifest_path.clone(),
            source,
        })?;
    let manifest =
        serde_json::from_str::<Manifest>(&source).map_err(|source| RunnerError::ParseManifest {
            path: options.manifest_path.clone(),
            source,
        })?;
    validate_manifest(&options.manifest_path, &manifest)?;
    let profile_name = options.profile.clone().unwrap_or_else(|| "full".to_owned());
    let profile = manifest
        .profiles
        .iter()
        .find(|profile| profile.name == profile_name)
        .ok_or_else(|| RunnerError::UnknownProfile {
            path: options.manifest_path.clone(),
            profile: profile_name.clone(),
        })?;
    let selected_gate_ids = profile
        .gates
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();

    let repo_root =
        options
            .repo_root
            .canonicalize()
            .map_err(|source| RunnerError::InvalidRepositoryRoot {
                path: options.repo_root.clone(),
                source,
            })?;

    let mut gates = Vec::with_capacity(profile.gates.len());
    for gate in manifest.gates {
        if !selected_gate_ids.contains(&gate.id) {
            continue;
        }
        let working_directory = resolve_working_directory(&repo_root, &gate)?;
        let outcome = execute_gate(&gate, &working_directory);
        gates.push(result_for_gate(&gate, working_directory, outcome));
    }

    let success = gates.iter().all(|gate| gate.success);
    Ok(QualityReport {
        schema: REPORT_SCHEMA,
        profile: profile_name,
        success,
        exit_code: if success { 0 } else { 1 },
        gates,
    })
}

/// Serialize a report for the process stdout contract.
pub fn render_json(report: &QualityReport) -> Result<String, RunnerError> {
    report.json().map_err(RunnerError::SerializeReport)
}

fn validate_manifest(path: &Path, manifest: &Manifest) -> Result<(), RunnerError> {
    if manifest.schema != "omnirepo.quality-manifest.v1" || manifest.version != 1 {
        return Err(RunnerError::InvalidManifest {
            path: path.to_owned(),
            reason: "schema must be omnirepo.quality-manifest.v1 version 1".to_owned(),
        });
    }
    let mut gate_ids = std::collections::HashSet::new();
    for gate in &manifest.gates {
        if gate.id.is_empty()
            || gate.kind != "gate"
            || gate.toolchain.is_empty()
            || gate.failure_identity.is_empty()
            || gate.working_directory.is_empty()
            || gate.argv.is_empty()
            || gate.argv[0].is_empty()
        {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("gate {:?} has incomplete metadata", gate.id),
            });
        }
        if !gate_ids.insert(&gate.id) {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("gate ID {:?} is duplicated", gate.id),
            });
        }
        if gate.timeout_seconds == Some(0) {
            return Err(RunnerError::InvalidGateLimits {
                gate_id: gate.id.clone(),
                reason: "timeout_seconds must be greater than zero".to_owned(),
            });
        }
        if let Some(limit) = gate.output_limit_bytes
            && limit < OUTPUT_TRUNCATION_MARKER.len() * 2
        {
            return Err(RunnerError::InvalidGateLimits {
                gate_id: gate.id.clone(),
                reason: format!(
                    "output_limit_bytes must be at least {}",
                    OUTPUT_TRUNCATION_MARKER.len() * 2
                ),
            });
        }
    }

    let mut profile_names = std::collections::HashSet::new();
    for profile in &manifest.profiles {
        if profile.kind != "profile" || profile.name.is_empty() || profile.gates.is_empty() {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("profile {:?} has incomplete metadata", profile.name),
            });
        }
        if !profile_names.insert(&profile.name) {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("profile name {:?} is duplicated", profile.name),
            });
        }
        let mut profile_gate_ids = std::collections::HashSet::new();
        for gate_id in &profile.gates {
            if !gate_ids.contains(gate_id) {
                return Err(RunnerError::InvalidManifest {
                    path: path.to_owned(),
                    reason: format!(
                        "profile {:?} names unknown gate {:?}",
                        profile.name, gate_id
                    ),
                });
            }
            if !profile_gate_ids.insert(gate_id) {
                return Err(RunnerError::InvalidManifest {
                    path: path.to_owned(),
                    reason: format!("profile {:?} repeats gate {:?}", profile.name, gate_id),
                });
            }
        }
    }
    for required in ["full", "stable", "msrv", "coverage"] {
        if !profile_names.iter().any(|name| name.as_str() == required) {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("profiles must define {required:?}"),
            });
        }
    }
    let full = manifest
        .profiles
        .iter()
        .find(|profile| profile.name == "full")
        .expect("required full profile was checked above");
    if full.gates.len() != manifest.gates.len()
        || full.gates.iter().any(|gate_id| !gate_ids.contains(gate_id))
    {
        return Err(RunnerError::InvalidManifest {
            path: path.to_owned(),
            reason: "full profile must select every gate exactly once".to_owned(),
        });
    }
    Ok(())
}

fn resolve_working_directory(
    repo_root: &Path,
    gate: &ManifestGate,
) -> Result<PathBuf, RunnerError> {
    let declared = Path::new(&gate.working_directory);
    if declared.is_absolute() {
        return Err(RunnerError::InvalidWorkingDirectory {
            gate_id: gate.id.clone(),
            path: declared.to_owned(),
            reason: "working_directory must be repository-relative".to_owned(),
        });
    }

    let candidate = repo_root.join(declared);
    let resolved =
        candidate
            .canonicalize()
            .map_err(|source| RunnerError::InvalidWorkingDirectory {
                gate_id: gate.id.clone(),
                path: candidate.clone(),
                reason: source.to_string(),
            })?;
    if !resolved.starts_with(repo_root) {
        return Err(RunnerError::InvalidWorkingDirectory {
            gate_id: gate.id.clone(),
            path: candidate,
            reason: "working_directory escapes repo_root".to_owned(),
        });
    }
    Ok(resolved)
}

struct GateOutcome {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    failure: Option<GateFailure>,
}

#[derive(Debug)]
struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_overflow: bool,
    stderr_overflow: bool,
}

impl From<Output> for GateOutcome {
    fn from(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
            failure: None,
        }
    }
}

fn execute_gate(gate: &ManifestGate, working_directory: &Path) -> GateOutcome {
    let mut command = Command::new(&gate.argv[0]);
    command
        .args(gate.argv.iter().skip(1))
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return GateOutcome {
                status: process_error_status(&error),
                stdout: Vec::new(),
                stderr: error.to_string().into_bytes(),
                failure: Some(GateFailure::Spawn {
                    message: error.to_string(),
                }),
            };
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let capture = Arc::new(Mutex::new(CapturedOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_overflow: false,
        stderr_overflow: false,
    }));
    let overflow = Arc::new(AtomicBool::new(false));
    let limit = gate
        .output_limit_bytes
        .unwrap_or(DEFAULT_OUTPUT_LIMIT_BYTES);
    let stream_limit = limit.saturating_sub(OUTPUT_TRUNCATION_MARKER.len() * 2) / 2;
    let stdout_thread = spawn_capture_thread(
        stdout,
        Arc::clone(&capture),
        Arc::clone(&overflow),
        stream_limit,
        true,
    );
    let stderr_thread = spawn_capture_thread(
        stderr,
        Arc::clone(&capture),
        Arc::clone(&overflow),
        stream_limit,
        false,
    );

    let timeout = gate
        .timeout_seconds
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_GATE_TIMEOUT);
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                terminate_process_tree(&mut child);
                break child
                    .wait()
                    .unwrap_or_else(|_| process_error_status(&error));
            }
        }
        if overflow.load(Ordering::Acquire) {
            terminate_process_tree(&mut child);
            break child
                .wait()
                .unwrap_or_else(|error| process_error_status(&error));
        }
        if Instant::now() >= deadline {
            timed_out = true;
            terminate_process_tree(&mut child);
            break child
                .wait()
                .unwrap_or_else(|error| process_error_status(&error));
        }
        thread::park_timeout(Duration::from_millis(10));
    };

    let stdout_result = stdout_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stdout capture thread panicked")));
    let stderr_result = stderr_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stderr capture thread panicked")));
    let captured = Arc::try_unwrap(capture)
        .expect("capture readers released their shared state")
        .into_inner()
        .expect("capture mutex is not poisoned");
    let overflowed = captured.stdout_overflow || captured.stderr_overflow;
    let capture_failure = stdout_result
        .err()
        .or_else(|| stderr_result.err())
        .map(|error| GateFailure::Capture {
            message: error.to_string(),
        });
    GateOutcome {
        status,
        stdout: captured.stdout,
        stderr: captured.stderr,
        failure: if timed_out {
            Some(GateFailure::Timeout)
        } else if overflowed {
            Some(GateFailure::OutputOverflow)
        } else {
            capture_failure
        },
    }
}

fn spawn_capture_thread<R>(
    mut reader: R,
    capture: Arc<Mutex<CapturedOutput>>,
    overflow: Arc<AtomicBool>,
    stream_limit: usize,
    stdout: bool,
) -> thread::JoinHandle<io::Result<()>>
where
    R: io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            let mut captured = capture.lock().expect("capture mutex is not poisoned");
            let target = if stdout {
                &mut captured.stdout
            } else {
                &mut captured.stderr
            };
            let remaining = stream_limit.saturating_sub(target.len());
            let take = remaining.min(read);
            target.extend_from_slice(&buffer[..take]);
            let is_overflow = take != read;
            if stdout {
                captured.stdout_overflow |= is_overflow;
            } else {
                captured.stderr_overflow |= is_overflow;
            }
            if is_overflow {
                overflow.store(true, Ordering::Release);
            }
        }
    })
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = child.id() as i32;
        // The child is placed in its own process group before spawn. Kill the
        // group first so descendants cannot keep output pipes open.
        unsafe {
            let _ = killpg(process_group, SIGKILL);
        }
    }
    let _ = child.kill();
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn killpg(pgrp: i32, sig: i32) -> i32;
}

fn process_error_status(error: &io::Error) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        std::process::ExitStatus::from_raw(error.raw_os_error().unwrap_or(1) << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;

        std::process::ExitStatus::from_raw(error.raw_os_error().unwrap_or(1) as u32)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" })
            .status()
            .expect("platform has a process status implementation")
    }
}

fn result_for_gate(
    gate: &ManifestGate,
    working_directory: PathBuf,
    output: impl Into<GateOutcome>,
) -> GateResult {
    let output = output.into();
    let success = output.status.success() && output.failure.is_none();
    let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.failure == Some(GateFailure::OutputOverflow) {
        if stdout.len() + OUTPUT_TRUNCATION_MARKER.len() <= DEFAULT_OUTPUT_LIMIT_BYTES {
            stdout.push_str(OUTPUT_TRUNCATION_MARKER);
        }
        if stderr.len() + OUTPUT_TRUNCATION_MARKER.len() <= DEFAULT_OUTPUT_LIMIT_BYTES {
            stderr.push_str(OUTPUT_TRUNCATION_MARKER);
        }
    }
    GateResult {
        id: gate.id.clone(),
        failure_identity: gate.failure_identity.clone(),
        toolchain: gate.toolchain.clone(),
        working_directory: working_directory.display().to_string(),
        exit_code: output.status.code(),
        success,
        stdout,
        stderr,
        failure: output.failure,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GateFailure, GateResult, Manifest, ManifestGate, ManifestProfile, QualityReport,
        REPORT_SCHEMA, RunnerError, execute_gate, process_error_status, render_json,
        resolve_working_directory, result_for_gate, validate_manifest,
    };
    use std::{
        fs, io,
        path::{Path, PathBuf},
        process::Output,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

    fn gate(working_directory: &str) -> ManifestGate {
        ManifestGate {
            id: "fixture".to_owned(),
            kind: "gate".to_owned(),
            toolchain: "fixture-rust".to_owned(),
            working_directory: working_directory.to_owned(),
            argv: vec!["fixture".to_owned()],
            failure_identity: "quality.fixture".to_owned(),
            timeout_seconds: None,
            output_limit_bytes: None,
        }
    }

    fn profiles() -> Vec<ManifestProfile> {
        ["full", "stable", "msrv", "coverage"]
            .into_iter()
            .map(|name| ManifestProfile {
                name: name.to_owned(),
                kind: "profile".to_owned(),
                gates: vec!["fixture".to_owned()],
            })
            .collect()
    }

    fn manifest(gates: Vec<ManifestGate>, profiles: Vec<ManifestProfile>) -> Manifest {
        Manifest {
            schema: "omnirepo.quality-manifest.v1".to_owned(),
            version: 1,
            gates,
            profiles,
        }
    }

    fn profile(name: &str, gates: &[&str]) -> ManifestProfile {
        ManifestProfile {
            name: name.to_owned(),
            kind: "profile".to_owned(),
            gates: gates.iter().map(ToString::to_string).collect(),
        }
    }

    fn root(name: &str) -> PathBuf {
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "omnirepo-quality-module-{name}-{}-{sequence}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create quality fixture root");
        root
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    fn valid_output(status: std::process::ExitStatus) -> Output {
        Output {
            status,
            stdout: b"stdout".to_vec(),
            stderr: b"stderr".to_vec(),
        }
    }

    struct GateMutation {
        field: &'static str,
        apply: fn(&mut ManifestGate),
    }

    struct ProfileMutation {
        field: &'static str,
        apply: fn(&mut ManifestProfile),
    }

    #[test]
    fn report_schema_is_stable() {
        assert_eq!(REPORT_SCHEMA, "omnirepo.quality-report.v1");
    }

    #[test]
    fn manifest_validation_rejects_incomplete_gate() {
        let mut fixture = gate(".");
        fixture.argv.clear();
        let manifest = Manifest {
            schema: "omnirepo.quality-manifest.v1".to_owned(),
            version: 1,
            gates: vec![fixture],
            profiles: profiles(),
        };

        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &manifest),
            Err(RunnerError::InvalidManifest { .. })
        ));
    }

    #[test]
    fn manifest_validation_rejects_wrong_schema_and_version() {
        let mut invalid_schema = manifest(vec![gate(".")], profiles());
        invalid_schema.schema = "other.schema".to_owned();
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &invalid_schema),
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("schema")
        ));

        let mut invalid_version = manifest(vec![gate(".")], profiles());
        invalid_version.version = 2;
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &invalid_version),
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("schema")
        ));
    }

    #[test]
    fn manifest_validation_rejects_each_incomplete_gate_field() {
        let cases = [
            GateMutation {
                field: "id",
                apply: |gate| gate.id.clear(),
            },
            GateMutation {
                field: "kind",
                apply: |gate| gate.kind.clear(),
            },
            GateMutation {
                field: "toolchain",
                apply: |gate| gate.toolchain.clear(),
            },
            GateMutation {
                field: "failure_identity",
                apply: |gate| gate.failure_identity.clear(),
            },
            GateMutation {
                field: "working_directory",
                apply: |gate| gate.working_directory.clear(),
            },
            GateMutation {
                field: "argv",
                apply: |gate| gate.argv.clear(),
            },
            GateMutation {
                field: "argv[0]",
                apply: |gate| gate.argv[0].clear(),
            },
        ];

        for case in cases {
            let mut fixture = gate(".");
            (case.apply)(&mut fixture);
            let result = validate_manifest(
                Path::new("fixture.json"),
                &manifest(vec![fixture], profiles()),
            );
            assert!(
                matches!(result, Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("incomplete metadata")),
                "field {} should be rejected",
                case.field
            );
        }
    }

    #[test]
    fn manifest_validation_rejects_zero_timeout_and_tiny_output_limit() {
        let mut zero_timeout = gate(".");
        zero_timeout.timeout_seconds = Some(0);
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &manifest(vec![zero_timeout], profiles())),
            Err(RunnerError::InvalidGateLimits { reason, .. }) if reason.contains("timeout_seconds")
        ));

        let mut tiny_limit = gate(".");
        tiny_limit.output_limit_bytes = Some(1);
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &manifest(vec![tiny_limit], profiles())),
            Err(RunnerError::InvalidGateLimits { reason, .. }) if reason.contains("output_limit_bytes")
        ));
    }

    #[test]
    fn manifest_validation_rejects_duplicate_gate_and_profile_names() {
        let duplicate_gate = manifest(vec![gate("."), gate(".")], profiles());
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &duplicate_gate),
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("gate ID")
        ));

        let duplicate_profile = manifest(
            vec![gate(".")],
            vec![
                profile("full", &["fixture"]),
                profile("full", &["fixture"]),
                profile("stable", &["fixture"]),
                profile("msrv", &["fixture"]),
                profile("coverage", &["fixture"]),
            ],
        );
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &duplicate_profile),
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("profile name")
        ));
    }

    #[test]
    fn manifest_validation_rejects_each_profile_shape_and_reference_error() {
        let cases = [
            ProfileMutation {
                field: "kind",
                apply: |profile| profile.kind.clear(),
            },
            ProfileMutation {
                field: "name",
                apply: |profile| profile.name.clear(),
            },
            ProfileMutation {
                field: "gates",
                apply: |profile| profile.gates.clear(),
            },
        ];
        for case in cases {
            let mut changed = profile("stable", &["fixture"]);
            (case.apply)(&mut changed);
            let profiles = vec![
                profile("full", &["fixture"]),
                changed,
                profile("msrv", &["fixture"]),
                profile("coverage", &["fixture"]),
            ];
            let result = validate_manifest(
                Path::new("fixture.json"),
                &manifest(vec![gate(".")], profiles),
            );
            assert!(
                matches!(result, Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("incomplete metadata")),
                "profile field {} should be rejected",
                case.field
            );
        }

        let unknown_gate = manifest(
            vec![gate(".")],
            vec![
                profile("full", &["missing"]),
                profile("stable", &["fixture"]),
                profile("msrv", &["fixture"]),
                profile("coverage", &["fixture"]),
            ],
        );
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &unknown_gate),
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("unknown gate")
        ));

        let duplicate_reference = manifest(
            vec![gate(".")],
            vec![
                profile("full", &["fixture"]),
                profile("stable", &["fixture", "fixture"]),
                profile("msrv", &["fixture"]),
                profile("coverage", &["fixture"]),
            ],
        );
        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &duplicate_reference),
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("repeats gate")
        ));
    }

    #[test]
    fn manifest_validation_requires_every_named_profile() {
        for required in ["full", "stable", "msrv", "coverage"] {
            let profiles = ["full", "stable", "msrv", "coverage"]
                .into_iter()
                .filter(|name| *name != required)
                .map(|name| profile(name, &["fixture"]))
                .collect();
            let result = validate_manifest(
                Path::new("fixture.json"),
                &manifest(vec![gate(".")], profiles),
            );
            assert!(
                matches!(result, Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains(required)),
                "missing {required} must be reported"
            );
        }
    }

    #[test]
    fn full_profile_must_select_every_gate() {
        let result = validate_manifest(
            Path::new("fixture.json"),
            &manifest(
                vec![
                    gate("."),
                    ManifestGate {
                        id: "second".to_owned(),
                        ..gate(".")
                    },
                ],
                vec![
                    profile("full", &["fixture"]),
                    profile("stable", &["fixture"]),
                    profile("msrv", &["fixture"]),
                    profile("coverage", &["fixture"]),
                ],
            ),
        );
        assert!(matches!(
            result,
            Err(RunnerError::InvalidManifest { reason, .. }) if reason.contains("every gate")
        ));
    }

    #[test]
    fn manifest_validation_rejects_duplicate_profile_gate() {
        let manifest = Manifest {
            schema: "omnirepo.quality-manifest.v1".to_owned(),
            version: 1,
            gates: vec![gate(".")],
            profiles: vec![
                ManifestProfile {
                    name: "full".to_owned(),
                    kind: "profile".to_owned(),
                    gates: vec!["fixture".to_owned(), "fixture".to_owned()],
                },
                ManifestProfile {
                    name: "stable".to_owned(),
                    kind: "profile".to_owned(),
                    gates: vec!["fixture".to_owned()],
                },
                ManifestProfile {
                    name: "msrv".to_owned(),
                    kind: "profile".to_owned(),
                    gates: vec!["fixture".to_owned()],
                },
                ManifestProfile {
                    name: "coverage".to_owned(),
                    kind: "profile".to_owned(),
                    gates: vec!["fixture".to_owned()],
                },
            ],
        };

        assert!(matches!(
            validate_manifest(Path::new("fixture.json"), &manifest),
            Err(RunnerError::InvalidManifest { .. })
        ));
    }

    #[test]
    fn working_directory_cannot_escape_repository_root() {
        let root = std::env::temp_dir().join(format!(
            "omnirepo-quality-module-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create test root");
        let result = resolve_working_directory(&root, &gate("../"));
        let _ = std::fs::remove_dir_all(&root);

        assert!(matches!(
            result,
            Err(RunnerError::InvalidWorkingDirectory { .. })
        ));
    }

    #[test]
    fn working_directory_rejects_absolute_and_missing_paths() {
        let root = root("working-directory");
        let absolute = ManifestGate {
            working_directory: root.display().to_string(),
            ..gate(".")
        };
        assert!(matches!(
            resolve_working_directory(&root, &absolute),
            Err(RunnerError::InvalidWorkingDirectory { reason, .. }) if reason.contains("repository-relative")
        ));

        let missing = gate("missing");
        assert!(matches!(
            resolve_working_directory(&root, &missing),
            Err(RunnerError::InvalidWorkingDirectory { .. })
        ));
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn working_directory_rejects_symlink_outside_repository_root() {
        use std::os::unix::fs::symlink;

        let repository_root = root("symlink");
        let outside = root("outside");
        let link = repository_root.join("link");
        symlink(&outside, &link).expect("create outside symlink");
        let linked = gate("link");
        assert!(matches!(
            resolve_working_directory(&repository_root, &linked),
            Err(RunnerError::InvalidWorkingDirectory { reason, .. }) if reason.contains("escapes")
        ));
        cleanup(&repository_root);
        cleanup(&outside);
    }

    #[test]
    fn spawn_failure_is_preserved_as_a_failed_gate() {
        let root = root("spawn-failure");
        let gate = ManifestGate {
            argv: vec![root.join("does-not-exist").display().to_string()],
            ..gate(".")
        };
        let output = execute_gate(&gate, &root);
        let result = result_for_gate(&gate, root.clone(), output);
        assert!(!result.success);
        assert!(result.exit_code.is_some());
        assert!(result.stderr.contains("No such file") || result.stderr.contains("os error"));
        assert!(matches!(result.failure, Some(GateFailure::Spawn { .. })));
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn signal_termination_has_no_exit_code() {
        let root = root("signal");
        let gate = ManifestGate {
            argv: vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "kill -TERM $$".to_owned(),
            ],
            ..gate(".")
        };
        let output = execute_gate(&gate, &root);
        assert_eq!(output.status.code(), None);
        let result = result_for_gate(&gate, root.clone(), output);
        assert!(!result.success);
        assert_eq!(result.exit_code, None);
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_gate_output_is_lossy_but_inspectable() {
        let root = root("non-utf8");
        let gate = ManifestGate {
            argv: vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "printf '\\377'; printf '\\376' >&2".to_owned(),
            ],
            ..gate(".")
        };
        let output = execute_gate(&gate, &root);
        let result = result_for_gate(&gate, root.clone(), output);
        assert_eq!(result.stdout, "�");
        assert_eq!(result.stderr, "�");
        cleanup(&root);
    }

    #[test]
    fn process_error_status_is_non_success() {
        let error = io::Error::new(io::ErrorKind::NotFound, "missing");
        let status = process_error_status(&error);
        assert!(!status.success());
        assert_eq!(status.code(), Some(1));
    }

    #[test]
    fn error_display_and_source_are_stable() {
        let io_error = io::Error::new(io::ErrorKind::NotFound, "missing");
        let parse_error = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        let cases = [
            RunnerError::ReadManifest {
                path: PathBuf::from("manifest.json"),
                source: io::Error::new(io::ErrorKind::NotFound, "missing"),
            },
            RunnerError::ParseManifest {
                path: PathBuf::from("manifest.json"),
                source: serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json"),
            },
            RunnerError::InvalidManifest {
                path: PathBuf::from("manifest.json"),
                reason: "bad".to_owned(),
            },
            RunnerError::UnknownProfile {
                path: PathBuf::from("manifest.json"),
                profile: "missing".to_owned(),
            },
            RunnerError::InvalidRepositoryRoot {
                path: PathBuf::from("repo"),
                source: io::Error::new(io::ErrorKind::NotFound, "missing"),
            },
            RunnerError::InvalidWorkingDirectory {
                gate_id: "gate".to_owned(),
                path: PathBuf::from("nested"),
                reason: "bad".to_owned(),
            },
            RunnerError::SerializeReport(parse_error),
        ];
        for case in cases {
            assert!(!case.to_string().is_empty());
            match case {
                RunnerError::ReadManifest { .. }
                | RunnerError::ParseManifest { .. }
                | RunnerError::InvalidRepositoryRoot { .. }
                | RunnerError::SerializeReport(..) => {
                    assert!(std::error::Error::source(&case).is_some())
                }
                _ => assert!(std::error::Error::source(&case).is_none()),
            }
        }
        assert_eq!(io_error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn report_json_and_result_projection_are_stable() {
        let report = QualityReport {
            schema: REPORT_SCHEMA,
            profile: "stable".to_owned(),
            success: true,
            exit_code: 0,
            gates: vec![GateResult {
                id: "fixture".to_owned(),
                failure_identity: "quality.fixture".to_owned(),
                toolchain: "fixture-rust".to_owned(),
                working_directory: ".".to_owned(),
                exit_code: Some(0),
                success: true,
                stdout: "ok".to_owned(),
                stderr: String::new(),
                failure: None,
            }],
        };
        let json = render_json(&report).expect("report serializes");
        assert!(json.contains("omnirepo.quality-report.v1"));
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("report round trips");
        assert_eq!(parsed["schema"], REPORT_SCHEMA);
        assert_eq!(parsed["profile"], "stable");
        assert_eq!(parsed["gates"][0]["id"], "fixture");

        let status = if cfg!(unix) {
            std::process::Command::new("true")
                .status()
                .expect("true status")
        } else {
            std::process::Command::new("cmd")
                .args(["/C", "exit", "0"])
                .status()
                .expect("cmd status")
        };
        let projected = result_for_gate(&gate("."), PathBuf::from("."), valid_output(status));
        assert_eq!(projected.exit_code, Some(0));
        assert!(projected.success);
    }
}
