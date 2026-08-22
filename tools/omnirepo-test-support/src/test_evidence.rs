//! Structured evidence for unit, component, and end-to-end test runners.
//!
//! This module deliberately has no terminal-output side effects. Workers submit
//! typed events to EventRecorder; the recorder is the one aggregation and
//! persistence boundary. Events are sorted by their stable case identity, so
//! parallel completion order cannot change JSONL output or peer accounting.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ffi::CString;
use std::fmt::{self, Display, Formatter};
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::raw::{c_char, c_int, c_uint};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::ffi::OsStrExt;

use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer, IgnoredAny},
};

/// Version of the event records emitted by this crate.
pub const TEST_EVENT_SCHEMA: &str = "omnirepo.test-event.v1";
/// Version of the aggregate evidence bundle.
pub const EVIDENCE_BUNDLE_SCHEMA: &str = "omnirepo.test-evidence-bundle.v1";
/// Maximum combined diagnostic evidence retained in one bundle.
pub const MAX_EVIDENCE_BYTES: usize = 1024 * 1024;
/// Marker used when diagnostics are bounded.
pub const DIAGNOSTIC_TRUNCATION_MARKER: &str = "[diagnostic truncated]";
/// Marker used when a harness had to synthesize a terminal event.
pub const HARNESS_TERMINAL_MARKER: &str = "[harness terminal synthesized]";
const REDACTED_MARKER: &str = "[REDACTED]";
const CONTROL_SEQUENCE_MARKER: &str = "[control-sequence]";
const MAX_IDENTITY_BYTES: usize = 4096;

// `O_NOFOLLOW` and `O_DIRECTORY` are architecture-specific on Linux. The
// arm family reuses the arm `fcntl.h` values, while x86, riscv, s390x and
// loongarch use the asm-generic ones. A wrong value never fails loudly: the
// bit pattern simply means a different flag (the asm-generic `O_NOFOLLOW`
// is `O_LARGEFILE` on aarch64), so the open silently follows a symlink and
// voids the containment guarantee in canon/architecture/runtime-platform.md.
// An architecture whose values are not verified here fails the build instead
// of opening without no-follow.
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
const O_DIRECTORY: c_int = 0o40000;
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "riscv64",
        target_arch = "riscv32",
        target_arch = "s390x",
        target_arch = "loongarch64"
    )
))]
const O_DIRECTORY: c_int = 0o200000;
#[cfg(target_os = "linux")]
const O_CLOEXEC: c_int = 0o2000000;
// `O_NOFOLLOW` and `O_DIRECTORY` are architecture-specific on Linux. The
// arm family reuses the arm `fcntl.h` values, while x86, riscv, s390x and
// loongarch use the asm-generic ones. A wrong value never fails loudly: the
// bit pattern simply means a different flag (the asm-generic `O_NOFOLLOW`
// is `O_LARGEFILE` on aarch64), so the open silently follows a symlink and
// voids the containment guarantee in canon/architecture/runtime-platform.md.
// An architecture whose values are not verified here fails the build instead
// of opening without no-follow.
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
const O_NOFOLLOW: c_int = 0o100000;
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "riscv64",
        target_arch = "riscv32",
        target_arch = "s390x",
        target_arch = "loongarch64"
    )
))]
const O_NOFOLLOW: c_int = 0o400000;
#[cfg(all(
    target_os = "linux",
    not(any(
        any(target_arch = "aarch64", target_arch = "arm"),
        any(
            target_arch = "x86_64",
            target_arch = "x86",
            target_arch = "riscv64",
            target_arch = "riscv32",
            target_arch = "s390x",
            target_arch = "loongarch64"
        )
    ))
))]
const _UNVERIFIED_LINUX_ARCHITECTURE: () = compile_error!(
    "this Linux architecture has unverified O_NOFOLLOW/O_DIRECTORY values; \
add the architecture's exact fcntl.h values rather than building without \
no-follow containment"
);
#[cfg(target_os = "linux")]
const O_WRONLY: c_int = 0o1;
#[cfg(target_os = "linux")]
const O_CREAT: c_int = 0o100;
#[cfg(target_os = "linux")]
const O_EXCL: c_int = 0o200;
#[cfg(target_os = "linux")]
const AT_FDCWD: c_int = -100;

#[cfg(target_os = "macos")]
const O_DIRECTORY: c_int = 0x0010_0000;
#[cfg(target_os = "macos")]
const O_CLOEXEC: c_int = 0x0100_0000;
#[cfg(target_os = "macos")]
const O_NOFOLLOW: c_int = 0x0000_0100;
#[cfg(target_os = "macos")]
const O_WRONLY: c_int = 0x0000_0001;
#[cfg(target_os = "macos")]
const O_CREAT: c_int = 0x0000_0200;
#[cfg(target_os = "macos")]
const O_EXCL: c_int = 0x0000_0800;
#[cfg(target_os = "macos")]
const AT_FDCWD: c_int = -2;

#[cfg(any(target_os = "linux", target_os = "macos"))]
unsafe extern "C" {
    fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, ...) -> c_int;
    fn mkdirat(dirfd: c_int, pathname: *const c_char, mode: c_uint) -> c_int;
}

/// A typed failure from evidence collection, validation, or safe persistence.
#[derive(Debug)]
pub enum EvidenceError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    InvalidOutcome,
    DuplicateCase(String),
    DuplicateTerminal(String),
    MissingStart(String),
    RecorderFinalized,
    InvalidArtifactPath(PathBuf),
    ArtifactEscapesRoot(PathBuf),
    ArtifactSymlink(PathBuf),
    Json(serde_json::Error),
    Io(io::Error),
    Poisoned,
}

impl Display for EvidenceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => {
                write!(f, "invalid evidence field {field:?}: {reason}")
            }
            Self::InvalidOutcome => f.write_str("a start outcome cannot be terminal"),
            Self::DuplicateCase(id) => write!(f, "duplicate test case correlation: {id}"),
            Self::DuplicateTerminal(id) => {
                write!(f, "duplicate terminal event for correlation: {id}")
            }
            Self::MissingStart(id) => write!(f, "terminal event has no start: {id}"),
            Self::RecorderFinalized => f.write_str("evidence recorder is already finalized"),
            Self::InvalidArtifactPath(path) => {
                write!(f, "invalid evidence artifact path: {}", path.display())
            }
            Self::ArtifactEscapesRoot(path) => {
                write!(f, "evidence artifact escapes its root: {}", path.display())
            }
            Self::ArtifactSymlink(path) => {
                write!(f, "evidence artifact crosses a symlink: {}", path.display())
            }
            Self::Json(error) => write!(f, "evidence JSON error: {error}"),
            Self::Io(error) => write!(f, "evidence I/O error: {error}"),
            Self::Poisoned => f.write_str("evidence recorder lock is poisoned"),
        }
    }
}

impl std::error::Error for EvidenceError {}

impl From<io::Error> for EvidenceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for EvidenceError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// The stable source/plan/configuration identity carried by every event.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct SourcePlanConfig {
    pub source: String,
    pub plan: String,
    pub config: String,
}

impl SourcePlanConfig {
    pub fn new(
        source: impl Into<String>,
        plan: impl Into<String>,
        config: impl Into<String>,
    ) -> Result<Self, EvidenceError> {
        let source = checked_field("source", source.into())?;
        let plan = checked_field("plan", plan.into())?;
        let config = checked_field("config", config.into())?;
        Ok(Self {
            source,
            plan,
            config,
        })
    }

    fn validate(&self) -> Result<(), EvidenceError> {
        checked_field("source", self.source.clone())?;
        checked_field("plan", self.plan.clone())?;
        checked_field("config", self.config.clone())?;
        Ok(())
    }
}

/// Stable identity shared by a step's start and terminal events.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct TestIdentity {
    pub case_id: String,
    pub suite: String,
    pub repository: String,
    pub stage: String,
    #[serde(flatten)]
    pub source_plan_config: SourcePlanConfig,
    pub attempt: u32,
    pub seed: u64,
    /// A classification such as unit, component, e2e, or harness.
    pub command: String,
}

impl TestIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        case_id: impl Into<String>,
        suite: impl Into<String>,
        repository: impl Into<String>,
        stage: impl Into<String>,
        source_plan_config: SourcePlanConfig,
        attempt: u32,
        seed: u64,
        command: impl Into<String>,
    ) -> Result<Self, EvidenceError> {
        source_plan_config.validate()?;
        Ok(Self {
            case_id: checked_field("case_id", case_id.into())?,
            suite: checked_field("suite", suite.into())?,
            repository: checked_field("repository", repository.into())?,
            stage: checked_field("stage", stage.into())?,
            source_plan_config,
            attempt,
            seed,
            command: checked_field("command", command.into())?,
        })
    }

    fn validate(&self) -> Result<(), EvidenceError> {
        checked_field("case_id", self.case_id.clone())?;
        checked_field("suite", self.suite.clone())?;
        checked_field("repository", self.repository.clone())?;
        checked_field("stage", self.stage.clone())?;
        self.source_plan_config.validate()?;
        checked_field("command", self.command.clone())?;
        Ok(())
    }

    /// Return a deterministic, readable correlation key for this case attempt.
    pub fn correlation_id(&self) -> String {
        format!(
            "{}-{:016x}",
            self.case_id,
            stable_hash(&[
                &self.case_id,
                &self.suite,
                &self.repository,
                &self.stage,
                &self.source_plan_config.source,
                &self.source_plan_config.plan,
                &self.source_plan_config.config,
                &self.attempt.to_string(),
                &self.seed.to_string(),
                &self.command,
            ])
        )
    }
}

/// Whether a record begins a step or terminalizes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Start,
    Terminal,
}

/// Terminal outcome for one test step.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Started,
    Passed,
    Failed,
    Skipped,
    HarnessFailure,
}

impl Outcome {
    fn is_terminal(self) -> bool {
        !matches!(self, Self::Started)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::HarnessFailure => "harness_failure",
        }
    }
}

/// A safe pointer to a replay and its retained artifact.
#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct ArtifactReference {
    pub path: Option<String>,
    pub replay_id: Option<String>,
}

impl ArtifactReference {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn new(
        path: impl Into<PathBuf>,
        replay_id: impl Into<String>,
    ) -> Result<Self, EvidenceError> {
        let path = safe_relative_path(&path.into())?;
        let replay_id = checked_replay_id(replay_id.into())?;
        Ok(Self {
            path: Some(path.to_string_lossy().into_owned()),
            replay_id: Some(replay_id),
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref().map(Path::new)
    }
}

/// A structured event. All identity fields are repeated on both event kinds
/// so a single JSONL line remains self-describing after filtering.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct TestEvent {
    pub schema: String,
    pub event_id: String,
    pub correlation_id: String,
    pub event_kind: EventKind,
    pub terminal: bool,
    #[serde(flatten)]
    pub identity: TestIdentity,
    pub outcome: Outcome,
    pub duration_ms: u64,
    pub artifact: ArtifactReference,
    pub diagnostic: Option<String>,
}

impl fmt::Debug for TestEvent {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestEvent")
            .field("schema", &self.schema)
            .field("event_id", &self.event_id)
            .field("correlation_id", &self.correlation_id)
            .field("event_kind", &self.event_kind)
            .field("terminal", &self.terminal)
            .field("identity", &self.identity)
            .field("outcome", &self.outcome)
            .field("duration_ms", &self.duration_ms)
            .field("artifact", &self.artifact)
            .field(
                "diagnostic",
                &self.diagnostic.as_ref().map(|_| REDACTED_MARKER),
            )
            .finish()
    }
}

impl TestEvent {
    fn start(identity: TestIdentity, artifact: ArtifactReference) -> Self {
        let correlation_id = identity.correlation_id();
        Self {
            schema: TEST_EVENT_SCHEMA.to_owned(),
            event_id: format!("{correlation_id}/start"),
            correlation_id,
            event_kind: EventKind::Start,
            terminal: false,
            identity,
            outcome: Outcome::Started,
            duration_ms: 0,
            artifact,
            diagnostic: None,
        }
    }

    fn terminal(
        identity: TestIdentity,
        artifact: ArtifactReference,
        outcome: Outcome,
        duration_ms: u64,
        diagnostic: Option<String>,
    ) -> Result<Self, EvidenceError> {
        if !outcome.is_terminal() {
            return Err(EvidenceError::InvalidOutcome);
        }
        let correlation_id = identity.correlation_id();
        Ok(Self {
            schema: TEST_EVENT_SCHEMA.to_owned(),
            event_id: format!("{correlation_id}/terminal"),
            correlation_id,
            event_kind: EventKind::Terminal,
            terminal: true,
            identity,
            outcome,
            duration_ms,
            artifact,
            diagnostic,
        })
    }
}

/// Redacts known secrets, URI userinfo, authentication values, and terminal
/// control sequences before a diagnostic enters an event.
#[derive(Clone, Default)]
pub struct DiagnosticRedactor {
    secrets: Vec<String>,
}

impl fmt::Debug for DiagnosticRedactor {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiagnosticRedactor")
            .field("secret_count", &self.secrets.len())
            .finish()
    }
}

impl DiagnosticRedactor {
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut secrets = secrets
            .into_iter()
            .map(Into::into)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        secrets.dedup();
        Self { secrets }
    }

    pub fn sanitize(&self, input: &str) -> SanitizedDiagnostic {
        sanitize_diagnostic(input, &self.secrets)
    }
}

/// Result of applying the evidence redaction rules to one diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedDiagnostic {
    pub text: String,
    pub redacted: bool,
    pub truncated: bool,
    pub control_escaped: bool,
    pub non_utf8: bool,
}

/// Sanitized stdout and stderr retained under one shared byte bound.
///
/// The channel order is stable: `stdout` bytes precede `stderr` bytes when a
/// caller forms one combined diagnostic. If the combined text is truncated,
/// one canonical marker is appended to the first channel whose text is cut.
/// `combined_bytes` includes that marker and is always at most the requested
/// bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedChannels {
    pub stdout: SanitizedDiagnostic,
    pub stderr: SanitizedDiagnostic,
    pub combined_bytes: usize,
}

/// Apply the same redaction and control-sequence rules used by event recording.
pub fn sanitize_diagnostic(input: &str, secrets: &[String]) -> SanitizedDiagnostic {
    sanitize_text(input, secrets, MAX_EVIDENCE_BYTES, false)
}

/// Sanitize complete stdout and stderr channels under one shared byte bound.
///
/// Each input is decoded with replacement for invalid UTF-8, then passed
/// through the same redaction and control escaping rules as ordinary
/// diagnostics. The byte budget is shared in stdout-then-stderr order. When
/// the sanitized text does not fit, the first channel that is cut receives
/// one [`DIAGNOSTIC_TRUNCATION_MARKER`]; the marker bytes are part of the
/// shared budget. The bound must leave room for that marker and cannot exceed
/// [`MAX_EVIDENCE_BYTES`].
pub fn sanitize_channels(
    redactor: &DiagnosticRedactor,
    stdout: &[u8],
    stderr: &[u8],
    max_bytes: usize,
) -> Result<SanitizedChannels, EvidenceError> {
    if !(DIAGNOSTIC_TRUNCATION_MARKER.len()..=MAX_EVIDENCE_BYTES).contains(&max_bytes) {
        return Err(EvidenceError::InvalidField {
            field: "max_bytes",
            reason: "channel bound must fit the canonical marker and one-MiB maximum",
        });
    }

    let stdout_non_utf8 = std::str::from_utf8(stdout).is_err();
    let stderr_non_utf8 = std::str::from_utf8(stderr).is_err();
    let stdout_text = String::from_utf8_lossy(stdout);
    let stderr_text = String::from_utf8_lossy(stderr);
    let mut sanitized_stdout = sanitize_text(
        stdout_text.as_ref(),
        &redactor.secrets,
        usize::MAX,
        stdout_non_utf8,
    );
    let mut sanitized_stderr = sanitize_text(
        stderr_text.as_ref(),
        &redactor.secrets,
        usize::MAX,
        stderr_non_utf8,
    );

    let total = sanitized_stdout.text.len() + sanitized_stderr.text.len();
    if total <= max_bytes {
        return Ok(SanitizedChannels {
            stdout: sanitized_stdout,
            stderr: sanitized_stderr,
            combined_bytes: total,
        });
    }

    let content_budget = max_bytes - DIAGNOSTIC_TRUNCATION_MARKER.len();
    let stdout_limit = content_budget.min(sanitized_stdout.text.len());
    let stdout_truncated = stdout_limit < sanitized_stdout.text.len();
    let remaining = content_budget - stdout_limit;
    let stderr_limit = remaining.min(sanitized_stderr.text.len());
    let stderr_truncated = stderr_limit < sanitized_stderr.text.len();
    let (stdout_text, _) = truncate_utf8(&sanitized_stdout.text, stdout_limit);
    let (stderr_text, _) = truncate_utf8(&sanitized_stderr.text, stderr_limit);
    sanitized_stdout.text = stdout_text;
    sanitized_stderr.text = stderr_text;
    sanitized_stdout.truncated |= stdout_truncated;
    sanitized_stderr.truncated |= stderr_truncated;

    if stdout_truncated {
        sanitized_stdout.text.push_str(DIAGNOSTIC_TRUNCATION_MARKER);
    } else {
        sanitized_stderr.text.push_str(DIAGNOSTIC_TRUNCATION_MARKER);
    }
    let combined_bytes = sanitized_stdout.text.len() + sanitized_stderr.text.len();
    debug_assert_eq!(combined_bytes, max_bytes);
    Ok(SanitizedChannels {
        stdout: sanitized_stdout,
        stderr: sanitized_stderr,
        combined_bytes,
    })
}

fn sanitize_text(
    input: &str,
    secrets: &[String],
    max_bytes: usize,
    non_utf8: bool,
) -> SanitizedDiagnostic {
    let mut redacted = false;
    let mut text = input.to_owned();

    for secret in secrets {
        if text.contains(secret) {
            text = text.replace(secret, REDACTED_MARKER);
            redacted = true;
        }
    }

    let (uri_text, uri_redacted) = redact_uri_userinfo(&text);
    text = uri_text;
    redacted |= uri_redacted;
    let (credential_text, credential_redacted) = redact_credential_values(&text);
    text = credential_text;
    redacted |= credential_redacted;
    let (control_text, control_escaped) = escape_control_sequences(&text);
    text = control_text;
    redacted |= control_escaped;
    let (text, truncated) = truncate_utf8(&text, max_bytes);

    SanitizedDiagnostic {
        text,
        redacted,
        truncated,
        control_escaped,
        non_utf8,
    }
}

/// Expected and observed peer accounting retained in the bundle.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PeerAccounting {
    pub expected_case_ids: Vec<String>,
    pub terminal_case_ids: Vec<String>,
    pub missing_case_ids: Vec<String>,
    pub terminal_outcomes: BTreeMap<String, Outcome>,
}

impl PeerAccounting {
    fn validate(&self) -> Result<(), EvidenceError> {
        let expected = validate_case_id_list("expected_case_ids", &self.expected_case_ids)?;
        let terminal = validate_case_id_list("terminal_case_ids", &self.terminal_case_ids)?;
        let missing = validate_case_id_list("missing_case_ids", &self.missing_case_ids)?;

        if expected.is_empty() {
            return Err(EvidenceError::InvalidField {
                field: "expected_case_ids",
                reason: "peer accounting must contain at least one expected case",
            });
        }
        if !terminal.is_disjoint(&missing) {
            return Err(EvidenceError::InvalidField {
                field: "peer_accounting",
                reason: "terminal and missing case partitions must be disjoint",
            });
        }
        if expected != terminal.union(&missing).cloned().collect() {
            return Err(EvidenceError::InvalidField {
                field: "peer_accounting",
                reason: "terminal and missing cases must partition expected cases",
            });
        }
        if self.terminal_outcomes.len() != terminal.len() {
            return Err(EvidenceError::InvalidField {
                field: "terminal_outcomes",
                reason: "one terminal outcome is required for each terminal case",
            });
        }
        for correlation in self.terminal_outcomes.keys() {
            checked_field("correlation_id", correlation.clone())?;
        }
        if self
            .terminal_outcomes
            .values()
            .any(|outcome| !outcome.is_terminal())
        {
            return Err(EvidenceError::InvalidOutcome);
        }
        Ok(())
    }
}

/// Concise terminal projection. It never contains raw diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminalProjection {
    pub outcome: Outcome,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub harness_failures: usize,
    pub missing: usize,
    pub artifact_path: Option<String>,
    pub replay_id: Option<String>,
}

impl TerminalProjection {
    fn validate(&self) -> Result<(), EvidenceError> {
        if !self.outcome.is_terminal() {
            return Err(EvidenceError::InvalidOutcome);
        }
        let terminal_count = self
            .passed
            .checked_add(self.failed)
            .and_then(|count| count.checked_add(self.skipped))
            .and_then(|count| count.checked_add(self.harness_failures))
            .ok_or(EvidenceError::InvalidField {
                field: "projection",
                reason: "terminal counts overflow",
            })?;
        if terminal_count == 0 {
            return Err(EvidenceError::InvalidField {
                field: "projection",
                reason: "projection must contain at least one terminal outcome",
            });
        }
        if self.missing > self.harness_failures {
            return Err(EvidenceError::InvalidField {
                field: "missing",
                reason: "missing peers must be represented by harness failures",
            });
        }
        let expected_outcome = if self.harness_failures > 0 || self.missing > 0 {
            Outcome::HarnessFailure
        } else if self.failed > 0 {
            Outcome::Failed
        } else if self.passed == 0 && self.skipped > 0 {
            Outcome::Skipped
        } else {
            Outcome::Passed
        };
        if self.outcome != expected_outcome {
            return Err(EvidenceError::InvalidField {
                field: "outcome",
                reason: "projection outcome does not match its counts",
            });
        }
        ArtifactReference::from_parts(self.artifact_path.clone(), self.replay_id.clone())?;
        Ok(())
    }

    pub fn render_quiet(&self) -> String {
        let mut result = format!(
            "test evidence: {} (passed={}, failed={}, skipped={}, harness_failures={}, missing={})",
            self.outcome.label(),
            self.passed,
            self.failed,
            self.skipped,
            self.harness_failures,
            self.missing
        );
        if let Some(path) = &self.artifact_path {
            result.push_str(" evidence=");
            result.push_str(path);
        }
        if let Some(replay_id) = &self.replay_id {
            result.push_str(" replay=");
            result.push_str(replay_id);
        }
        result
    }
}

fn validate_case_id_list(
    field: &'static str,
    values: &[String],
) -> Result<BTreeSet<String>, EvidenceError> {
    let mut unique = BTreeSet::new();
    for value in values {
        checked_field("case_id", value.clone())?;
        if !unique.insert(value.clone()) {
            return Err(EvidenceError::InvalidField {
                field,
                reason: "case IDs must be unique",
            });
        }
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(EvidenceError::InvalidField {
            field,
            reason: "case IDs must be in strict deterministic order",
        });
    }
    Ok(unique)
}

/// Persisted, deterministic collection of all events and their accounting.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvidenceBundle {
    pub schema: String,
    pub events: Vec<TestEvent>,
    pub peer_accounting: PeerAccounting,
    pub projection: TerminalProjection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourcePlanConfigWire {
    source: String,
    plan: String,
    config: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestIdentityWire {
    case_id: String,
    suite: String,
    repository: String,
    stage: String,
    source: String,
    plan: String,
    config: String,
    attempt: u32,
    seed: u64,
    command: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReferenceWire {
    path: Option<String>,
    replay_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestEventWire {
    schema: String,
    event_id: String,
    correlation_id: String,
    event_kind: EventKind,
    terminal: bool,
    case_id: String,
    suite: String,
    repository: String,
    stage: String,
    source: String,
    plan: String,
    config: String,
    attempt: u32,
    seed: u64,
    command: String,
    outcome: Outcome,
    duration_ms: u64,
    artifact: ArtifactReferenceWire,
    diagnostic: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerAccountingWire {
    expected_case_ids: Vec<String>,
    terminal_case_ids: Vec<String>,
    missing_case_ids: Vec<String>,
    terminal_outcomes: BTreeMap<String, Outcome>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalProjectionWire {
    outcome: Outcome,
    passed: usize,
    failed: usize,
    skipped: usize,
    harness_failures: usize,
    missing: usize,
    artifact_path: Option<String>,
    replay_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceBundleWire {
    schema: String,
    events: Vec<TestEvent>,
    peer_accounting: PeerAccounting,
    projection: TerminalProjection,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonlSummary {
    schema: String,
    record: String,
    peer_accounting: PeerAccounting,
    projection: TerminalProjection,
}

#[derive(Deserialize)]
struct JsonlMarker {
    record: Option<String>,
    #[serde(flatten)]
    _ignored: BTreeMap<String, IgnoredAny>,
}

impl<'de> Deserialize<'de> for SourcePlanConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SourcePlanConfigWire::deserialize(deserializer)?;
        Self::new(wire.source, wire.plan, wire.config).map_err(de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for TestIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TestIdentityWire::deserialize(deserializer)?;
        let source_plan_config = SourcePlanConfig::new(wire.source, wire.plan, wire.config)
            .map_err(de::Error::custom)?;
        Self::new(
            wire.case_id,
            wire.suite,
            wire.repository,
            wire.stage,
            source_plan_config,
            wire.attempt,
            wire.seed,
            wire.command,
        )
        .map_err(de::Error::custom)
    }
}

impl ArtifactReference {
    fn from_parts(path: Option<String>, replay_id: Option<String>) -> Result<Self, EvidenceError> {
        match (path, replay_id) {
            (None, None) => Ok(Self::none()),
            (Some(path), Some(replay_id)) => Self::new(path, replay_id),
            _ => Err(EvidenceError::InvalidField {
                field: "artifact",
                reason: "path and replay_id must be set together",
            }),
        }
    }

    fn validate(&self) -> Result<(), EvidenceError> {
        Self::from_parts(self.path.clone(), self.replay_id.clone()).map(|_| ())
    }
}

impl<'de> Deserialize<'de> for ArtifactReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ArtifactReferenceWire::deserialize(deserializer)?;
        Self::from_parts(wire.path, wire.replay_id).map_err(de::Error::custom)
    }
}

impl TestEvent {
    fn from_wire(wire: TestEventWire) -> Result<Self, EvidenceError> {
        let source_plan_config = SourcePlanConfig::new(wire.source, wire.plan, wire.config)?;
        let identity = TestIdentity::new(
            wire.case_id,
            wire.suite,
            wire.repository,
            wire.stage,
            source_plan_config,
            wire.attempt,
            wire.seed,
            wire.command,
        )?;
        let event = Self {
            schema: wire.schema,
            event_id: wire.event_id,
            correlation_id: wire.correlation_id,
            event_kind: wire.event_kind,
            terminal: wire.terminal,
            identity,
            outcome: wire.outcome,
            duration_ms: wire.duration_ms,
            artifact: ArtifactReference::from_parts(wire.artifact.path, wire.artifact.replay_id)?,
            diagnostic: wire.diagnostic,
        };
        event.validate_shape()?;
        Ok(event)
    }

    fn validate_shape(&self) -> Result<(), EvidenceError> {
        if self.schema != TEST_EVENT_SCHEMA {
            return Err(EvidenceError::InvalidField {
                field: "schema",
                reason: "unsupported test event schema",
            });
        }
        self.identity.validate()?;
        self.artifact.validate()?;
        if self.correlation_id != self.identity.correlation_id() {
            return Err(EvidenceError::InvalidField {
                field: "correlation_id",
                reason: "correlation does not match the event identity",
            });
        }
        let expected_event_id = match self.event_kind {
            EventKind::Start => format!("{}/start", self.correlation_id),
            EventKind::Terminal => format!("{}/terminal", self.correlation_id),
        };
        if self.event_id != expected_event_id {
            return Err(EvidenceError::InvalidField {
                field: "event_id",
                reason: "event ID does not match the event kind and correlation",
            });
        }
        if self.terminal != matches!(self.event_kind, EventKind::Terminal) {
            return Err(EvidenceError::InvalidField {
                field: "terminal",
                reason: "terminal flag does not match event kind",
            });
        }
        match self.event_kind {
            EventKind::Start => {
                if self.outcome != Outcome::Started {
                    return Err(EvidenceError::InvalidField {
                        field: "outcome",
                        reason: "start events must use the started outcome",
                    });
                }
                if self.duration_ms != 0 || self.diagnostic.is_some() {
                    return Err(EvidenceError::InvalidField {
                        field: "start",
                        reason: "start events cannot carry terminal data",
                    });
                }
            }
            EventKind::Terminal => {
                if !self.outcome.is_terminal() {
                    return Err(EvidenceError::InvalidOutcome);
                }
            }
        }
        if self.diagnostic.as_deref().is_some_and(|diagnostic| {
            diagnostic
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        }) {
            return Err(EvidenceError::InvalidField {
                field: "diagnostic",
                reason: "control characters must be escaped before persistence",
            });
        }
        if self
            .diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.len() > MAX_EVIDENCE_BYTES)
        {
            return Err(EvidenceError::InvalidField {
                field: "diagnostic",
                reason: "persisted diagnostic exceeds the one-MiB evidence bound",
            });
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for TestEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TestEventWire::deserialize(deserializer)?;
        Self::from_wire(wire).map_err(de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for PeerAccounting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PeerAccountingWire::deserialize(deserializer)?;
        let accounting = Self {
            expected_case_ids: wire.expected_case_ids,
            terminal_case_ids: wire.terminal_case_ids,
            missing_case_ids: wire.missing_case_ids,
            terminal_outcomes: wire.terminal_outcomes,
        };
        accounting.validate().map_err(de::Error::custom)?;
        Ok(accounting)
    }
}

impl<'de> Deserialize<'de> for TerminalProjection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TerminalProjectionWire::deserialize(deserializer)?;
        let artifact = ArtifactReference::from_parts(wire.artifact_path, wire.replay_id)
            .map_err(de::Error::custom)?;
        let projection = Self {
            outcome: wire.outcome,
            passed: wire.passed,
            failed: wire.failed,
            skipped: wire.skipped,
            harness_failures: wire.harness_failures,
            missing: wire.missing,
            artifact_path: artifact.path,
            replay_id: artifact.replay_id,
        };
        projection.validate().map_err(de::Error::custom)?;
        Ok(projection)
    }
}

impl<'de> Deserialize<'de> for EvidenceBundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EvidenceBundleWire::deserialize(deserializer)?;
        let bundle = Self {
            schema: wire.schema,
            events: wire.events,
            peer_accounting: wire.peer_accounting,
            projection: wire.projection,
        };
        bundle.validate().map_err(de::Error::custom)?;
        Ok(bundle)
    }
}

impl EvidenceBundle {
    /// Serialize events and a final summary record as parseable JSON Lines.
    pub fn to_jsonl(&self) -> Result<String, EvidenceError> {
        self.validate()?;
        let mut lines = Vec::with_capacity(self.events.len() + 1);
        for event in &self.events {
            lines.push(serde_json::to_string(event)?);
        }
        let summary = JsonlSummary {
            schema: EVIDENCE_BUNDLE_SCHEMA.to_owned(),
            record: "summary".to_owned(),
            peer_accounting: self.peer_accounting.clone(),
            projection: self.projection.clone(),
        };
        lines.push(serde_json::to_string(&summary)?);
        Ok(format!("{}\n", lines.join("\n")))
    }

    /// Parse JSONL written by EvidenceBundle::to_jsonl.
    pub fn from_jsonl(contents: &str) -> Result<Self, EvidenceError> {
        let mut lines = contents.split('\n').collect::<Vec<_>>();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        if lines.is_empty() || lines.iter().any(|line| line.trim().is_empty()) {
            return Err(EvidenceError::InvalidField {
                field: "JSONL",
                reason: "blank records are not allowed",
            });
        }

        let mut events = Vec::new();
        let mut summary = None;
        let final_index = lines.len() - 1;
        for (index, line) in lines.into_iter().enumerate() {
            let marker: JsonlMarker = serde_json::from_str(line)?;
            if marker.record.as_deref() == Some("summary") {
                if index == final_index {
                    let summary_record: JsonlSummary = serde_json::from_str(line)?;
                    if summary_record.record != "summary" {
                        return Err(EvidenceError::InvalidField {
                            field: "record",
                            reason: "unsupported JSONL summary record",
                        });
                    }
                    if summary.is_some() {
                        return Err(EvidenceError::InvalidField {
                            field: "record",
                            reason: "JSONL has more than one summary record",
                        });
                    }
                    summary = Some(summary_record);
                } else {
                    return Err(EvidenceError::InvalidField {
                        field: "record",
                        reason: "summary must be the final JSONL record",
                    });
                }
            } else {
                let event: TestEvent = serde_json::from_str(line)?;
                if index == final_index {
                    return Err(EvidenceError::InvalidField {
                        field: "record",
                        reason: "the final JSONL record must be the summary",
                    });
                }
                events.push(event);
            }
        }
        if events.is_empty() {
            return Err(EvidenceError::InvalidField {
                field: "events",
                reason: "JSONL must contain at least one event",
            });
        }
        let summary = summary.ok_or(EvidenceError::InvalidField {
            field: "record",
            reason: "JSONL has no final summary record",
        })?;
        if summary.record != "summary" {
            return Err(EvidenceError::InvalidField {
                field: "record",
                reason: "unsupported JSONL summary record",
            });
        }
        let bundle = Self {
            schema: summary.schema.clone(),
            events,
            peer_accounting: summary.peer_accounting,
            projection: summary.projection,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn validate(&self) -> Result<(), EvidenceError> {
        if self.schema != EVIDENCE_BUNDLE_SCHEMA {
            return Err(EvidenceError::InvalidField {
                field: "schema",
                reason: "unsupported evidence bundle schema",
            });
        }
        if self.events.is_empty() {
            return Err(EvidenceError::InvalidField {
                field: "events",
                reason: "evidence must contain at least one event",
            });
        }

        let mut starts = BTreeMap::new();
        let mut terminals = BTreeMap::new();
        let mut start_artifacts = BTreeMap::new();
        let mut terminal_artifacts = BTreeMap::new();
        let mut event_ids = BTreeSet::new();
        for (index, event) in self.events.iter().enumerate() {
            event.validate_shape()?;
            if !event_ids.insert(event.event_id.clone()) {
                return Err(EvidenceError::InvalidField {
                    field: "event_id",
                    reason: "event IDs must be unique",
                });
            }
            if index > 0 && event_order(&self.events[index - 1], event).is_gt() {
                return Err(EvidenceError::InvalidField {
                    field: "events",
                    reason: "events are not in deterministic order",
                });
            }
            match event.event_kind {
                EventKind::Start => {
                    if starts
                        .insert(event.correlation_id.clone(), event.identity.clone())
                        .is_some()
                    {
                        return Err(EvidenceError::DuplicateCase(event.correlation_id.clone()));
                    }
                    start_artifacts.insert(event.correlation_id.clone(), event.artifact.clone());
                }
                EventKind::Terminal => {
                    if terminals
                        .insert(event.correlation_id.clone(), event.identity.clone())
                        .is_some()
                    {
                        return Err(EvidenceError::DuplicateTerminal(
                            event.correlation_id.clone(),
                        ));
                    }
                    terminal_artifacts.insert(event.correlation_id.clone(), event.artifact.clone());
                }
            }
        }
        if starts.keys().ne(terminals.keys()) {
            return Err(EvidenceError::InvalidField {
                field: "events",
                reason: "every start must have one terminal event",
            });
        }
        if starts
            .iter()
            .any(|(correlation, identity)| terminals.get(correlation) != Some(identity))
        {
            return Err(EvidenceError::InvalidField {
                field: "identity",
                reason: "start and terminal identities must match",
            });
        }
        if starts.iter().any(|(correlation, _)| {
            start_artifacts.get(correlation) != terminal_artifacts.get(correlation)
        }) {
            return Err(EvidenceError::InvalidField {
                field: "artifact",
                reason: "start and terminal artifact references must match",
            });
        }

        let expected_accounting = peer_accounting(&starts, &self.events);
        self.peer_accounting.validate()?;
        if self.peer_accounting != expected_accounting {
            return Err(EvidenceError::InvalidField {
                field: "peer_accounting",
                reason: "summary accounting does not match validated events",
            });
        }
        let expected_projection = projection(&self.events, &expected_accounting);
        self.projection.validate()?;
        if self.projection != expected_projection {
            return Err(EvidenceError::InvalidField {
                field: "projection",
                reason: "summary projection does not match validated events",
            });
        }
        validate_persisted_diagnostic_budget(&self.events)?;
        Ok(())
    }

    pub fn write_jsonl(&self, path: impl AsRef<Path>) -> Result<(), EvidenceError> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .ok_or_else(|| EvidenceError::InvalidArtifactPath(path.into()))?;
        fs::create_dir_all(parent)?;
        let mut file = File::create(path)?;
        file.write_all(self.to_jsonl()?.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
}

struct RecorderState {
    expected: BTreeMap<String, TestIdentity>,
    events: BTreeMap<String, TestEvent>,
    finalized: bool,
}

/// The single synchronized writer/aggregator used by worker threads.
#[derive(Clone)]
pub struct EventRecorder {
    state: Arc<Mutex<RecorderState>>,
    redactor: DiagnosticRedactor,
}

impl Default for EventRecorder {
    fn default() -> Self {
        Self::new(DiagnosticRedactor::default())
    }
}

impl EventRecorder {
    pub fn new(redactor: DiagnosticRedactor) -> Self {
        Self {
            state: Arc::new(Mutex::new(RecorderState {
                expected: BTreeMap::new(),
                events: BTreeMap::new(),
                finalized: false,
            })),
            redactor,
        }
    }

    /// Register a peer before worker dispatch. A missing peer is represented by
    /// a synthetic harness start/terminal pair during finalization.
    pub fn expect(&self, identity: TestIdentity) -> Result<(), EvidenceError> {
        identity.validate()?;
        let correlation = identity.correlation_id();
        let mut state = self.lock()?;
        if state.finalized {
            return Err(EvidenceError::RecorderFinalized);
        }
        if state
            .expected
            .insert(correlation.clone(), identity)
            .is_some()
        {
            return Err(EvidenceError::DuplicateCase(correlation));
        }
        Ok(())
    }

    /// Emit a start event and return a guard that terminalizes the step.
    pub fn start(
        &self,
        identity: TestIdentity,
        artifact: ArtifactReference,
    ) -> Result<StepGuard, EvidenceError> {
        identity.validate()?;
        artifact.validate()?;
        let correlation = identity.correlation_id();
        let mut state = self.lock()?;
        if state.finalized {
            return Err(EvidenceError::RecorderFinalized);
        }
        if state.events.contains_key(&format!("{correlation}/start")) {
            return Err(EvidenceError::DuplicateCase(correlation));
        }
        state
            .expected
            .entry(correlation.clone())
            .or_insert_with(|| identity.clone());
        state.events.insert(
            format!("{correlation}/start"),
            TestEvent::start(identity.clone(), artifact.clone()),
        );
        drop(state);
        Ok(StepGuard {
            recorder: self.clone(),
            identity,
            artifact,
            started_at: Instant::now(),
            finished: false,
        })
    }

    fn terminal(
        &self,
        identity: TestIdentity,
        artifact: ArtifactReference,
        outcome: Outcome,
        duration_ms: u64,
        diagnostic: Option<&str>,
    ) -> Result<(), EvidenceError> {
        identity.validate()?;
        artifact.validate()?;
        let correlation = identity.correlation_id();
        let terminal_id = format!("{correlation}/terminal");
        let mut state = self.lock()?;
        if state.finalized {
            return Err(EvidenceError::RecorderFinalized);
        }
        if !state.events.contains_key(&format!("{correlation}/start")) {
            return Err(EvidenceError::MissingStart(correlation));
        }
        if state.events.contains_key(&terminal_id) {
            return Err(EvidenceError::DuplicateTerminal(correlation));
        }
        let sanitized = diagnostic.map(|value| self.redactor.sanitize(value).text);
        let event = TestEvent::terminal(identity, artifact, outcome, duration_ms, sanitized)?;
        state.events.insert(terminal_id, event);
        Ok(())
    }

    /// Finalize all starts, synthesizing harness failures for dropped or
    /// missing workers, then sort and bound the complete bundle.
    pub fn finalize(&self) -> Result<EvidenceBundle, EvidenceError> {
        let mut state = self.lock()?;
        if state.finalized {
            return Err(EvidenceError::RecorderFinalized);
        }
        state.finalized = true;

        let expected = state.expected.clone();
        for (correlation, identity) in &expected {
            let start_id = format!("{correlation}/start");
            let terminal_id = format!("{correlation}/terminal");
            let artifact = state
                .events
                .get(&start_id)
                .map(|event| event.artifact.clone())
                .unwrap_or_else(ArtifactReference::none);
            state
                .events
                .entry(start_id)
                .or_insert_with(|| TestEvent::start(identity.clone(), ArtifactReference::none()));
            if let Entry::Vacant(slot) = state.events.entry(terminal_id) {
                let terminal = TestEvent::terminal(
                    identity.clone(),
                    artifact,
                    Outcome::HarnessFailure,
                    0,
                    Some(HARNESS_TERMINAL_MARKER.to_owned()),
                )?;
                slot.insert(terminal);
            }
        }

        // This also covers callers that emitted a start without first calling
        // expect; expected was populated by start.
        let starts = state
            .events
            .values()
            .filter(|event| event.event_kind == EventKind::Start)
            .cloned()
            .collect::<Vec<_>>();
        for start in starts {
            let terminal_id = format!("{}/terminal", start.correlation_id);
            if let Entry::Vacant(slot) = state.events.entry(terminal_id) {
                let terminal = TestEvent::terminal(
                    start.identity,
                    start.artifact,
                    Outcome::HarnessFailure,
                    0,
                    Some(HARNESS_TERMINAL_MARKER.to_owned()),
                )?;
                slot.insert(terminal);
            }
        }

        let mut events = state.events.values().cloned().collect::<Vec<_>>();
        events.sort_by(event_order);
        bound_diagnostics(&mut events);
        let peer_accounting = peer_accounting(&expected, &events);
        let projection = projection(&events, &peer_accounting);
        let bundle = EvidenceBundle {
            schema: EVIDENCE_BUNDLE_SCHEMA.to_owned(),
            events,
            peer_accounting,
            projection,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    fn lock(&self) -> Result<MutexGuard<'_, RecorderState>, EvidenceError> {
        self.state.lock().map_err(|_| EvidenceError::Poisoned)
    }
}

/// RAII handle that guarantees a terminal event even when the worker exits
/// early or forgets to report an assertion/harness failure.
pub struct StepGuard {
    recorder: EventRecorder,
    identity: TestIdentity,
    artifact: ArtifactReference,
    started_at: Instant,
    finished: bool,
}

impl StepGuard {
    pub fn finish(
        mut self,
        outcome: Outcome,
        diagnostic: Option<&str>,
    ) -> Result<(), EvidenceError> {
        let duration_ms = self
            .started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.finish_with_duration(outcome, duration_ms, diagnostic)
    }

    pub fn finish_with_duration(
        &mut self,
        outcome: Outcome,
        duration_ms: u64,
        diagnostic: Option<&str>,
    ) -> Result<(), EvidenceError> {
        if self.finished {
            return Err(EvidenceError::DuplicateTerminal(
                self.identity.correlation_id(),
            ));
        }
        self.recorder.terminal(
            self.identity.clone(),
            self.artifact.clone(),
            outcome,
            duration_ms,
            diagnostic,
        )?;
        self.finished = true;
        Ok(())
    }

    pub fn pass(mut self) -> Result<(), EvidenceError> {
        self.finish_with_duration(
            Outcome::Passed,
            self.started_at.elapsed().as_millis() as u64,
            None,
        )
    }

    pub fn fail(mut self, diagnostic: impl AsRef<str>) -> Result<(), EvidenceError> {
        self.finish_with_duration(
            Outcome::Failed,
            self.started_at.elapsed().as_millis() as u64,
            Some(diagnostic.as_ref()),
        )
    }

    pub fn skip(mut self, diagnostic: impl AsRef<str>) -> Result<(), EvidenceError> {
        self.finish_with_duration(Outcome::Skipped, 0, Some(diagnostic.as_ref()))
    }

    pub fn harness_failure(mut self, diagnostic: impl AsRef<str>) -> Result<(), EvidenceError> {
        self.finish_with_duration(
            Outcome::HarnessFailure,
            self.started_at.elapsed().as_millis() as u64,
            Some(diagnostic.as_ref()),
        )
    }
}

impl Drop for StepGuard {
    fn drop(&mut self) {
        if !self.finished {
            let duration_ms = self
                .started_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64;
            let _ = self.recorder.terminal(
                self.identity.clone(),
                self.artifact.clone(),
                Outcome::HarnessFailure,
                duration_ms,
                Some(HARNESS_TERMINAL_MARKER),
            );
            self.finished = true;
        }
    }
}

/// Result retained by execute_case without printing worker output.
#[derive(Clone, Eq, PartialEq)]
pub struct CaseExecution {
    pub outcome: Outcome,
    pub body_diagnostic: Option<String>,
    pub cleanup_diagnostic: Option<String>,
}

impl fmt::Debug for CaseExecution {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaseExecution")
            .field("outcome", &self.outcome)
            .field(
                "body_diagnostic",
                &self.body_diagnostic.as_ref().map(|_| REDACTED_MARKER),
            )
            .field(
                "cleanup_diagnostic",
                &self.cleanup_diagnostic.as_ref().map(|_| REDACTED_MARKER),
            )
            .finish()
    }
}

impl CaseExecution {
    pub fn success(&self) -> bool {
        self.outcome == Outcome::Passed
    }
}

/// Execute a test body and cleanup closure while preserving cleanup after a
/// body assertion/panic. Both failure channels become structured evidence.
pub fn execute_case<B, C>(
    recorder: &EventRecorder,
    identity: TestIdentity,
    artifact: ArtifactReference,
    body: B,
    cleanup: C,
) -> Result<CaseExecution, EvidenceError>
where
    B: FnOnce() -> Result<(), String> + panic::UnwindSafe,
    C: FnOnce() -> Result<(), String> + panic::UnwindSafe,
{
    let guard = recorder.start(identity, artifact)?;
    let body_result = panic::catch_unwind(AssertUnwindSafe(body));
    let cleanup_result = panic::catch_unwind(AssertUnwindSafe(cleanup));
    let body_diagnostic = match body_result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(payload) => Some(format!("harness panic: {}", panic_message(payload))),
    };
    let cleanup_diagnostic = match cleanup_result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(payload) => Some(format!("cleanup panic: {}", panic_message(payload))),
    };
    let outcome = if cleanup_diagnostic.is_some()
        || body_diagnostic
            .as_deref()
            .is_some_and(|diagnostic| diagnostic.starts_with("harness panic:"))
    {
        Outcome::HarnessFailure
    } else if body_diagnostic.is_some() {
        Outcome::Failed
    } else {
        Outcome::Passed
    };
    let diagnostic = join_diagnostics(body_diagnostic.as_deref(), cleanup_diagnostic.as_deref());
    guard.finish(outcome, diagnostic.as_deref())?;
    Ok(CaseExecution {
        outcome,
        body_diagnostic,
        cleanup_diagnostic,
    })
}

/// A safe artifact writer. All paths are relative to one authority root and
/// existing symlink components are rejected before any bytes are written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, EvidenceError> {
        let root = root.into();
        let root = prepare_authority_root(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve(&self, relative: impl AsRef<Path>) -> Result<PathBuf, EvidenceError> {
        let relative = safe_relative_path(relative.as_ref())?;
        let path = self.root.join(&relative);
        self.check_components(&relative, &path)?;
        Ok(path)
    }

    pub fn write_bytes(
        &self,
        relative: impl AsRef<Path>,
        bytes: &[u8],
    ) -> Result<ArtifactReference, EvidenceError> {
        let relative = safe_relative_path(relative.as_ref())?;
        self.resolve(&relative)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        secure_write_relative(&self.root, &relative, bytes)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let path = self.resolve(&relative)?;
            if let Some(parent) = path.parent() {
                create_directory_chain(parent)?;
                self.check_components(&relative, &path)?;
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        let relative_text = relative.to_string_lossy();
        let replay_id = format!("replay-{:016x}", stable_hash(&[relative_text.as_ref()]));
        ArtifactReference::new(relative, replay_id)
    }

    pub fn write_bundle(
        &self,
        relative: impl AsRef<Path>,
        bundle: &EvidenceBundle,
    ) -> Result<ArtifactReference, EvidenceError> {
        let jsonl = bundle.to_jsonl()?;
        self.write_bytes(relative, jsonl.as_bytes())
    }

    fn check_components(&self, relative: &Path, full_path: &Path) -> Result<(), EvidenceError> {
        let root_metadata = fs::symlink_metadata(&self.root)?;
        if root_metadata.file_type().is_symlink() {
            return Err(EvidenceError::ArtifactSymlink(self.root.clone()));
        }
        if !root_metadata.is_dir() {
            return Err(EvidenceError::InvalidArtifactPath(self.root.clone()));
        }
        let mut current = self.root.clone();
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(part) = component else {
                return Err(EvidenceError::InvalidArtifactPath(relative.to_owned()));
            };
            current.push(part);
            if let Ok(metadata) = fs::symlink_metadata(&current) {
                if metadata.file_type().is_symlink() {
                    return Err(EvidenceError::ArtifactSymlink(current));
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(EvidenceError::InvalidArtifactPath(current));
                }
            }
        }
        if !full_path.starts_with(&self.root) {
            return Err(EvidenceError::ArtifactEscapesRoot(full_path.to_owned()));
        }
        Ok(())
    }
}

fn prepare_authority_root(root: &Path) -> Result<PathBuf, EvidenceError> {
    if root.as_os_str().is_empty() {
        return Err(EvidenceError::InvalidArtifactPath(root.to_owned()));
    }
    let absolute = if root.is_absolute() {
        root.to_owned()
    } else {
        std::env::current_dir()?.join(root)
    };
    let mut missing = Vec::new();
    let mut cursor = absolute.clone();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(EvidenceError::ArtifactSymlink(cursor));
                }
                if !metadata.is_dir() {
                    return Err(EvidenceError::InvalidArtifactPath(cursor));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor.clone());
                let Some(parent) = cursor.parent() else {
                    return Err(EvidenceError::InvalidArtifactPath(cursor));
                };
                if parent == cursor {
                    return Err(EvidenceError::InvalidArtifactPath(cursor));
                }
                cursor = parent.to_owned();
            }
            Err(error) => return Err(error.into()),
        }
    }

    for path in missing.iter().rev() {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(EvidenceError::ArtifactSymlink(path.clone()));
                }
                if !metadata.is_dir() {
                    return Err(EvidenceError::InvalidArtifactPath(path.clone()));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(path)?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let mut ancestor = absolute.clone();
    loop {
        let metadata = fs::symlink_metadata(&ancestor)?;
        if metadata.file_type().is_symlink() {
            return Err(EvidenceError::ArtifactSymlink(ancestor));
        }
        if !metadata.is_dir() {
            return Err(EvidenceError::InvalidArtifactPath(ancestor));
        }
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if parent == ancestor {
            break;
        }
        ancestor = parent.to_owned();
    }
    Ok(absolute.canonicalize()?)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn create_directory_chain(path: &Path) -> Result<(), EvidenceError> {
    let mut missing = Vec::new();
    let mut cursor = path.to_owned();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(EvidenceError::ArtifactSymlink(cursor));
                }
                if !metadata.is_dir() {
                    return Err(EvidenceError::InvalidArtifactPath(cursor));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor.clone());
                let Some(parent) = cursor.parent() else {
                    return Err(EvidenceError::InvalidArtifactPath(cursor));
                };
                if parent == cursor {
                    return Err(EvidenceError::InvalidArtifactPath(cursor));
                }
                cursor = parent.to_owned();
            }
            Err(error) => return Err(error.into()),
        }
    }
    for path in missing.iter().rev() {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(EvidenceError::ArtifactSymlink(path.clone()));
                }
                if !metadata.is_dir() {
                    return Err(EvidenceError::InvalidArtifactPath(path.clone()));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn secure_write_relative(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), EvidenceError> {
    let mut directory = secure_open_directory(root)?;
    let components = relative.components().collect::<Vec<_>>();
    let Some((last, parents)) = components.split_last() else {
        return Err(EvidenceError::InvalidArtifactPath(relative.to_owned()));
    };
    for component in parents {
        let Component::Normal(part) = component else {
            return Err(EvidenceError::InvalidArtifactPath(relative.to_owned()));
        };
        let name = CString::new(part.as_bytes())
            .map_err(|_| EvidenceError::InvalidArtifactPath(relative.to_owned()))?;
        let mut child_fd = unsafe {
            openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
            )
        };
        if child_fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            let result = unsafe { mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error().into());
            }
            child_fd = unsafe {
                openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
                )
            };
        }
        if child_fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        directory = unsafe { OwnedFd::from_raw_fd(child_fd) };
    }
    let Component::Normal(part) = last else {
        return Err(EvidenceError::InvalidArtifactPath(relative.to_owned()));
    };
    let name = CString::new(part.as_bytes())
        .map_err(|_| EvidenceError::InvalidArtifactPath(relative.to_owned()))?;
    let file_fd = unsafe {
        openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
            0o600,
        )
    };
    if file_fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut file = File::from(unsafe { OwnedFd::from_raw_fd(file_fd) });
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn secure_open_directory(path: &Path) -> Result<OwnedFd, EvidenceError> {
    let root_name = CString::new("/").expect("static root path has no NUL");
    let root_fd = unsafe {
        openat(
            AT_FDCWD,
            root_name.as_ptr(),
            O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut directory = unsafe { OwnedFd::from_raw_fd(root_fd) };
    for component in path.components() {
        let part = match component {
            Component::RootDir => continue,
            Component::Normal(part) => part,
            _ => return Err(EvidenceError::InvalidArtifactPath(path.to_owned())),
        };
        let name = CString::new(part.as_bytes())
            .map_err(|_| EvidenceError::InvalidArtifactPath(path.to_owned()))?;
        let child_fd = unsafe {
            openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
            )
        };
        if child_fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        directory = unsafe { OwnedFd::from_raw_fd(child_fd) };
    }
    Ok(directory)
}

fn checked_field(field: &'static str, value: String) -> Result<String, EvidenceError> {
    if value.is_empty() {
        return Err(EvidenceError::InvalidField {
            field,
            reason: "value must not be empty",
        });
    }
    if value.len() > MAX_IDENTITY_BYTES {
        return Err(EvidenceError::InvalidField {
            field,
            reason: "value exceeds the bounded identity size",
        });
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(EvidenceError::InvalidField {
            field,
            reason: "control characters are not allowed",
        });
    }
    Ok(value)
}

fn checked_replay_id(value: String) -> Result<String, EvidenceError> {
    let value = checked_field("replay_id", value)?;
    if value == "." || value == ".." || value.contains('/') || value.contains('\\') {
        return Err(EvidenceError::InvalidField {
            field: "replay_id",
            reason: "replay IDs cannot contain path separators or traversal",
        });
    }
    Ok(value)
}

fn stable_hash(parts: &[&str]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn safe_relative_path(path: &Path) -> Result<PathBuf, EvidenceError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(EvidenceError::InvalidArtifactPath(path.to_owned()));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part)
                if !part
                    .to_string_lossy()
                    .chars()
                    .any(|character| character.is_control()) =>
            {
                clean.push(part)
            }
            _ => return Err(EvidenceError::InvalidArtifactPath(path.to_owned())),
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(EvidenceError::InvalidArtifactPath(path.to_owned()));
    }
    Ok(clean)
}

fn event_order(left: &TestEvent, right: &TestEvent) -> std::cmp::Ordering {
    left.identity
        .cmp(&right.identity)
        .then_with(|| left.event_kind.cmp(&right.event_kind))
        .then_with(|| left.event_id.cmp(&right.event_id))
}

fn bound_diagnostics(events: &mut [TestEvent]) {
    let total = events
        .iter()
        .filter_map(|event| event.diagnostic.as_ref())
        .map(String::len)
        .sum::<usize>();
    let reserve_marker = total > MAX_EVIDENCE_BYTES;
    let marker_bytes = if reserve_marker {
        DIAGNOSTIC_TRUNCATION_MARKER.len()
    } else {
        0
    };
    let mut remaining = MAX_EVIDENCE_BYTES.saturating_sub(marker_bytes);
    let mut truncation_emitted = false;
    for event in events.iter_mut() {
        let Some(diagnostic) = event.diagnostic.take() else {
            continue;
        };
        if remaining == 0 {
            continue;
        }
        if reserve_marker && diagnostic.len() > remaining {
            let prefix_limit = remaining.saturating_sub(marker_bytes);
            let (prefix, _) = truncate_utf8(&diagnostic, prefix_limit);
            event.diagnostic = Some(format!("{prefix}{DIAGNOSTIC_TRUNCATION_MARKER}"));
            truncation_emitted = true;
            remaining = 0;
        } else if diagnostic.len() <= remaining {
            remaining -= diagnostic.len();
            event.diagnostic = Some(diagnostic);
        } else {
            let (bounded, _) = truncate_utf8(&diagnostic, remaining);
            event.diagnostic = Some(bounded);
            remaining = 0;
        }
    }
    if reserve_marker && !truncation_emitted {
        // A diagnostic can end exactly at the reserved boundary. Replace its
        // tail so the bundle still states that later evidence was dropped.
        if let Some(event) = events
            .iter_mut()
            .rev()
            .find(|event| event.diagnostic.is_some())
        {
            let diagnostic = event.diagnostic.take().unwrap_or_default();
            if diagnostic.len() >= marker_bytes {
                let prefix_limit = diagnostic.len() - marker_bytes;
                let (prefix, _) = truncate_utf8(&diagnostic, prefix_limit);
                event.diagnostic = Some(format!("{prefix}{DIAGNOSTIC_TRUNCATION_MARKER}"));
            }
        }
    }
}

fn validate_persisted_diagnostic_budget(events: &[TestEvent]) -> Result<(), EvidenceError> {
    let total = events.iter().try_fold(0usize, |total, event| {
        let diagnostic_bytes = event.diagnostic.as_ref().map_or(0, String::len);
        total
            .checked_add(diagnostic_bytes)
            .ok_or(EvidenceError::InvalidField {
                field: "diagnostic",
                reason: "persisted diagnostic byte accounting overflow",
            })
    })?;
    if total > MAX_EVIDENCE_BYTES {
        return Err(EvidenceError::InvalidField {
            field: "diagnostic",
            reason: "combined persisted diagnostics exceed the one-MiB evidence bound",
        });
    }
    Ok(())
}

fn peer_accounting(
    expected: &BTreeMap<String, TestIdentity>,
    events: &[TestEvent],
) -> PeerAccounting {
    let mut expected_case_ids = expected
        .values()
        .map(|identity| identity.case_id.clone())
        .collect::<Vec<_>>();
    expected_case_ids.sort();
    let mut terminal_case_ids = Vec::new();
    let mut terminal_outcomes = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_kind == EventKind::Terminal)
    {
        terminal_case_ids.push(event.identity.case_id.clone());
        terminal_outcomes.insert(event.correlation_id.clone(), event.outcome);
    }
    let observed = terminal_outcomes.keys().collect::<BTreeSet<_>>();
    let mut missing_case_ids = expected
        .iter()
        .filter(|(correlation, _)| !observed.contains(correlation))
        .map(|(_, identity)| identity.case_id.clone())
        .collect::<Vec<_>>();
    terminal_case_ids.sort();
    missing_case_ids.sort();
    PeerAccounting {
        expected_case_ids,
        terminal_case_ids,
        missing_case_ids,
        terminal_outcomes,
    }
}

fn projection(events: &[TestEvent], accounting: &PeerAccounting) -> TerminalProjection {
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut harness_failures = 0;
    let mut artifact = None;
    let mut replay = None;
    for event in events
        .iter()
        .filter(|event| event.event_kind == EventKind::Terminal)
    {
        match event.outcome {
            Outcome::Passed => passed += 1,
            Outcome::Failed => failed += 1,
            Outcome::Skipped => skipped += 1,
            Outcome::HarnessFailure => harness_failures += 1,
            Outcome::Started => {}
        }
        if artifact.is_none() {
            artifact = event.artifact.path.clone();
        }
        if replay.is_none() {
            replay = event.artifact.replay_id.clone();
        }
    }
    let outcome = if harness_failures > 0 || !accounting.missing_case_ids.is_empty() {
        Outcome::HarnessFailure
    } else if failed > 0 {
        Outcome::Failed
    } else if passed == 0 && skipped > 0 {
        Outcome::Skipped
    } else {
        Outcome::Passed
    };
    TerminalProjection {
        outcome,
        passed,
        failed,
        skipped,
        harness_failures,
        missing: accounting.missing_case_ids.len(),
        artifact_path: artifact,
        replay_id: replay,
    }
}

fn truncate_utf8(input: &str, max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (input.to_owned(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (input[..end].to_owned(), true)
}

fn redact_uri_userinfo(input: &str) -> (String, bool) {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    let mut redacted = false;
    while let Some(relative) = input[cursor..].find("://") {
        let scheme_end = cursor + relative + 3;
        output.push_str(&input[cursor..scheme_end]);
        let authority_end = input[scheme_end..]
            .find(|character: char| {
                character.is_whitespace() || character == '/' || character == '\\'
            })
            .map_or(input.len(), |offset| scheme_end + offset);
        let authority = &input[scheme_end..authority_end];
        if let Some(at) = authority.rfind('@') {
            output.push_str(REDACTED_MARKER);
            output.push('@');
            output.push_str(&authority[at + 1..]);
            redacted = true;
        } else {
            output.push_str(authority);
        }
        cursor = authority_end;
        if cursor >= input.len() {
            break;
        }
    }
    output.push_str(&input[cursor..]);
    (output, redacted)
}

fn redact_credential_values(input: &str) -> (String, bool) {
    const MARKERS: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
    ];
    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    let mut redacted = false;
    while cursor < input.len() {
        let Some((marker_start, marker)) = MARKERS
            .iter()
            .filter_map(|marker| {
                lower[cursor..]
                    .find(marker)
                    .map(|offset| (cursor + offset, *marker))
            })
            .min_by_key(|(offset, _)| *offset)
        else {
            output.push_str(&input[cursor..]);
            break;
        };
        output.push_str(&input[cursor..marker_start]);
        let marker_end = marker_start + marker.len();
        let mut separator = marker_end;
        while separator < input.len() && input.as_bytes()[separator].is_ascii_whitespace() {
            separator += 1;
        }
        if separator >= input.len() || !matches!(input.as_bytes()[separator], b'=' | b':') {
            output.push_str(&input[marker_start..marker_end]);
            cursor = marker_end;
            continue;
        }
        output.push_str(&input[marker_start..=separator]);
        let mut value_start = separator + 1;
        while value_start < input.len() && input.as_bytes()[value_start].is_ascii_whitespace() {
            output.push(input.as_bytes()[value_start] as char);
            value_start += 1;
        }
        let quote = input
            .as_bytes()
            .get(value_start)
            .copied()
            .filter(|byte| *byte == b'\'' || *byte == b'"');
        if quote.is_some() {
            value_start += 1;
        }
        let mut value_end = value_start;
        if marker == "authorization" || marker == "proxy-authorization" {
            while value_end < input.len()
                && !matches!(input.as_bytes()[value_end], b'\n' | b'\r' | b',')
            {
                value_end += 1;
            }
        } else {
            while value_end < input.len()
                && !input.as_bytes()[value_end].is_ascii_whitespace()
                && !matches!(
                    input.as_bytes()[value_end],
                    b',' | b';' | b'&' | b')' | b']' | b'}'
                )
            {
                value_end += 1;
            }
        }
        if value_end > value_start {
            output.push_str(REDACTED_MARKER);
            redacted = true;
            if let Some(quote) = quote {
                if input.as_bytes().get(value_end) == Some(&quote) {
                    output.push(quote as char);
                    value_end += 1;
                }
            }
            cursor = value_end;
        } else {
            output.push_str(&input[marker_end..]);
            break;
        }
    }
    (output, redacted)
}

fn escape_control_sequences(input: &str) -> (String, bool) {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let mut changed = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == 0x1b {
            output.push_str(CONTROL_SEQUENCE_MARKER);
            changed = true;
            index += 1;
            if let Some(next) = bytes.get(index).copied() {
                index += 1;
                if next == b']' {
                    while index < bytes.len() {
                        if bytes[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                } else if next == b'[' {
                    while index < bytes.len() {
                        if (0x40..=0x7e).contains(&bytes[index]) {
                            index += 1;
                            break;
                        }
                        index += 1;
                    }
                } else if (0x40..=0x7e).contains(&next) {
                    // A two-byte escape is already consumed.
                } else {
                    while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                        index += 1;
                    }
                    if index < bytes.len() {
                        index += 1;
                    }
                }
            }
            continue;
        }
        if (byte < 0x20 && !matches!(byte, b'\n' | b'\r' | b'\t')) || byte == 0x7f {
            output.push_str(&format!("\\u{{{byte:04x}}}"));
            changed = true;
            index += 1;
            continue;
        }
        let character = input[index..]
            .chars()
            .next()
            .expect("index remains on a UTF-8 boundary");
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            output.push_str(&format!("\\u{{{:04x}}}", character as u32));
            changed = true;
        } else {
            output.push(character);
        }
        index += character.len_utf8();
    }
    (output, changed)
}

fn join_diagnostics(body: Option<&str>, cleanup: Option<&str>) -> Option<String> {
    match (body, cleanup) {
        (Some(body), Some(cleanup)) => Some(format!("body: {body}; cleanup: {cleanup}")),
        (Some(body), None) => Some(body.to_owned()),
        (None, Some(cleanup)) => Some(cleanup.to_owned()),
        (None, None) => None,
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(case_id: &str) -> TestIdentity {
        TestIdentity::new(
            case_id,
            "unit",
            "repo-a",
            "verify",
            SourcePlanConfig::new("source-a", "plan-a", "config-a").unwrap(),
            1,
            41,
            "unit",
        )
        .unwrap()
    }

    #[test]
    fn schemas_and_identity_fields_are_stable() {
        let recorder = EventRecorder::default();
        let guard = recorder
            .start(identity("case-a"), ArtifactReference::none())
            .unwrap();
        guard.pass().unwrap();
        let bundle = recorder.finalize().unwrap();
        assert_eq!(bundle.schema, EVIDENCE_BUNDLE_SCHEMA);
        assert_eq!(bundle.events.len(), 2);
        assert_eq!(bundle.events[0].schema, TEST_EVENT_SCHEMA);
        assert_eq!(bundle.events[0].identity.case_id, "case-a");
        assert_eq!(
            bundle.events[0].identity.source_plan_config.source,
            "source-a"
        );
        assert_eq!(bundle.events[1].outcome, Outcome::Passed);
    }

    #[test]
    fn dropped_guard_synthesizes_harness_terminal() {
        let recorder = EventRecorder::default();
        let guard = recorder
            .start(identity("dropped"), ArtifactReference::none())
            .unwrap();
        drop(guard);
        let bundle = recorder.finalize().unwrap();
        let terminal = bundle.events.iter().find(|event| event.terminal).unwrap();
        assert_eq!(terminal.outcome, Outcome::HarnessFailure);
        assert_eq!(
            terminal.diagnostic.as_deref(),
            Some(HARNESS_TERMINAL_MARKER)
        );
    }

    #[test]
    fn registered_peer_without_worker_keeps_complete_accounting() {
        let recorder = EventRecorder::default();
        recorder.expect(identity("never-dispatched")).unwrap();
        let bundle = recorder.finalize().unwrap();
        assert_eq!(bundle.events.len(), 2);
        assert_eq!(
            bundle.peer_accounting.missing_case_ids,
            Vec::<String>::new()
        );
        assert_eq!(bundle.projection.harness_failures, 1);
        assert_eq!(bundle.projection.missing, 0);
    }

    #[test]
    fn parallel_submission_is_sorted_by_identity_not_completion_order() {
        let recorder = EventRecorder::default();
        let left = recorder.clone();
        let right = recorder.clone();
        let first = std::thread::spawn(move || {
            let guard = left
                .start(identity("z-case"), ArtifactReference::none())
                .unwrap();
            guard.pass().unwrap();
        });
        let second = std::thread::spawn(move || {
            let guard = right
                .start(identity("a-case"), ArtifactReference::none())
                .unwrap();
            guard.pass().unwrap();
        });
        first.join().unwrap();
        second.join().unwrap();
        let bundle = recorder.finalize().unwrap();
        let cases = bundle
            .events
            .iter()
            .map(|event| event.identity.case_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(cases, ["a-case", "a-case", "z-case", "z-case"]);
    }

    #[test]
    fn redaction_covers_secrets_uri_userinfo_credentials_and_ansi() {
        let redactor = DiagnosticRedactor::new(["super-secret"]);
        let sanitized = redactor.sanitize(concat!(
            "tok",
            "en=super-secret https://alice:",
            "pw@example.test/path Authorization: Bearer abc\n\x1b[31mboom\x1b[0m"
        ));
        assert!(sanitized.redacted);
        assert!(!sanitized.text.contains("super-secret"));
        assert!(!sanitized.text.contains("alice:pw"));
        assert!(!sanitized.text.contains('\x1b'));
        assert!(sanitized.text.contains(CONTROL_SEQUENCE_MARKER));
    }

    #[test]
    fn combined_diagnostic_bound_is_one_mib_and_marks_truncation() {
        let recorder = EventRecorder::new(DiagnosticRedactor::default());
        for index in 0..4 {
            let guard = recorder
                .start(
                    identity(&format!("case-{index}")),
                    ArtifactReference::none(),
                )
                .unwrap();
            guard.fail("x".repeat(MAX_EVIDENCE_BYTES / 2)).unwrap();
        }
        let bundle = recorder.finalize().unwrap();
        let total = bundle
            .events
            .iter()
            .filter_map(|event| event.diagnostic.as_ref())
            .map(String::len)
            .sum::<usize>();
        assert!(total <= MAX_EVIDENCE_BYTES + DIAGNOSTIC_TRUNCATION_MARKER.len());
        assert!(bundle.events.iter().any(|event| {
            event
                .diagnostic
                .as_deref()
                .is_some_and(|text| text.contains(DIAGNOSTIC_TRUNCATION_MARKER))
        }));
    }

    #[test]
    fn jsonl_round_trip_preserves_order_and_projection() {
        let recorder = EventRecorder::default();
        recorder
            .start(identity("round-trip"), ArtifactReference::none())
            .unwrap()
            .pass()
            .unwrap();
        let bundle = recorder.finalize().unwrap();
        let parsed = EvidenceBundle::from_jsonl(&bundle.to_jsonl().unwrap()).unwrap();
        assert_eq!(parsed, bundle);
    }

    #[test]
    fn cleanup_runs_after_body_panic_and_reports_harness_failure() {
        let recorder = EventRecorder::default();
        let cleanup_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cleanup_marker = Arc::clone(&cleanup_ran);
        let execution = execute_case(
            &recorder,
            identity("cleanup-after-panic"),
            ArtifactReference::none(),
            || -> Result<(), String> { panic!("assertion exploded") },
            move || {
                cleanup_marker.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        assert!(cleanup_ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(execution.outcome, Outcome::HarnessFailure);
    }

    #[test]
    fn artifact_paths_reject_escape_and_symlink_components() {
        assert!(ArtifactReference::new("../escape", "replay").is_err());
        assert!(ArtifactReference::new("/absolute", "replay").is_err());
        // The system temp dir on macOS lives under /var/folders, and /var
        // is a symlink there; the store validates its root as symlink-free.
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        std::fs::create_dir_all(&base).expect("fixture base");
        let directory = tempfile::Builder::new()
            .prefix("evidence-src-root-")
            .tempdir_in(&base)
            .expect("fixture dir");
        let store = ArtifactStore::new(directory.path()).unwrap();
        let pointer = store.write_bytes("nested/evidence.jsonl", b"{}\n").unwrap();
        assert_eq!(pointer.path.as_deref(), Some("nested/evidence.jsonl"));
        assert!(store.resolve("../escape").is_err());
    }
}
