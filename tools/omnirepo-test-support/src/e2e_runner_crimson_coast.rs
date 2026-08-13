//! Clean-environment black-box runner mechanics.
//!
//! This module owns the process seam for executable acceptance journeys.  It
//! deliberately does not know what a journey means: callers provide a named
//! case, a controlled fixture executable, and the effects they expect.  The
//! runner supplies a hermetic fixture root, a sanitized environment, local
//! Git fixtures, process evidence, containment checks, and replay identity.
//!
//! The public product binary is intentionally not a dependency of this crate.
//! A caller may select an existing executable, or build a deterministic shell
//! fixture inside the temporary root.  This keeps runner tests honest while
//! the product journeys are implemented by their owning workstream.

#![allow(clippy::module_name_repetitions)]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tempfile::TempDir;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use super::lifecycle_fixture::{
    CleanupReport, DirtyGitState, FixtureError, FixtureOutcome, FixtureSpec, LifecycleFixture,
    RootKind,
};
use super::test_evidence::{
    ArtifactReference, ArtifactStore, DIAGNOSTIC_TRUNCATION_MARKER, DiagnosticRedactor,
    EventRecorder, EvidenceBundle, EvidenceError, MAX_EVIDENCE_BYTES, Outcome, SourcePlanConfig,
    TestIdentity, sanitize_channels,
};

/// Version of the runner evidence contract.
pub const E2E_RUNNER_CONTRACT_VERSION: &str = "clean-e2e-runner/v1";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_TERMINATION_GRACE: Duration = Duration::from_millis(100);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Errors raised while constructing, executing, or checking a black-box case.
#[derive(Debug)]
pub enum RunnerError {
    Fixture(FixtureError),
    Io(io::Error),
    InvalidCase(String),
    Build {
        binary: String,
        reason: String,
    },
    Spawn {
        binary: PathBuf,
        source: io::Error,
        root: PathBuf,
        report: Option<Box<RunReport>>,
    },
    Timeout {
        root: PathBuf,
        report: Box<RunReport>,
    },
    ExpectationFailed {
        details: String,
        report: Box<RunReport>,
    },
    Evidence(EvidenceError),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture(error) => write!(formatter, "E2E fixture error: {error}"),
            Self::Io(error) => write!(formatter, "E2E runner I/O error: {error}"),
            Self::InvalidCase(reason) => write!(formatter, "invalid E2E case: {reason}"),
            Self::Build { binary, reason } => {
                write!(
                    formatter,
                    "fixture binary {binary:?} could not be built: {reason}"
                )
            }
            Self::Spawn {
                binary,
                source,
                root,
                ..
            } => write!(
                formatter,
                "fixture binary {} could not start in {}: {source}",
                binary.display(),
                root.display()
            ),
            Self::Timeout { report, .. } => write!(
                formatter,
                "E2E case {} timed out (replay {}; evidence {})",
                report.case_id, report.replay_id, report.evidence_relative_path
            ),
            Self::ExpectationFailed { details, report } => write!(
                formatter,
                "E2E case {} failed expectations (replay {}; evidence {}): {details}",
                report.case_id, report.replay_id, report.evidence_relative_path
            ),
            Self::Evidence(error) => write!(formatter, "E2E evidence error: {error}"),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fixture(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Spawn { source, .. } => Some(source),
            Self::Evidence(error) => Some(error),
            Self::InvalidCase(_)
            | Self::Build { .. }
            | Self::Timeout { .. }
            | Self::ExpectationFailed { .. } => None,
        }
    }
}

impl RunnerError {
    /// Return retained evidence for failures that reached a fixture process.
    pub fn report(&self) -> Option<&RunReport> {
        match self {
            Self::Spawn {
                report: Some(report),
                ..
            }
            | Self::Timeout { report, .. }
            | Self::ExpectationFailed { report, .. } => Some(report),
            _ => None,
        }
    }

    pub fn is_spawn_failure(&self) -> bool {
        matches!(self, Self::Spawn { .. })
    }
}

impl From<FixtureError> for RunnerError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(error)
    }
}

impl From<io::Error> for RunnerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<EvidenceError> for RunnerError {
    fn from(error: EvidenceError) -> Self {
        Self::Evidence(error)
    }
}

/// Stable, path-safe case identity used by selection and replay.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CaseId(String);

impl CaseId {
    /// Parse a lowercase case slug.  Case IDs are part of evidence paths, so
    /// separators, traversal, and control text are rejected.
    pub fn parse(value: &str) -> Result<Self, RunnerError> {
        let valid = !value.is_empty()
            && value != "."
            && value != ".."
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            });
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(RunnerError::InvalidCase(format!(
                "case ID must be a non-empty lowercase slug: {value:?}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for CaseId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A fixture executable source selected by a journey definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureBinarySource {
    /// Build an executable shell fixture in the case root.  The script is
    /// invoked directly, with no shell-string command dispatch.
    ShellScript(String),
    /// Select an existing executable (usually the current Cargo-built product
    /// binary) and copy it into the case root before invocation.
    Existing(PathBuf),
}

/// Description of the executable used by one case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureBinarySpec {
    name: String,
    source: FixtureBinarySource,
}

/// Lossless identity for a path relative to one controlled root.
///
/// The textual display is intentionally separate from identity.  On Unix a
/// filename is an arbitrary byte sequence, so a lossy UTF-8 conversion cannot
/// be used as a map key or as an assertion target.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RelativePathIdentity {
    encoding: PathIdentityEncoding,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum PathIdentityEncoding {
    UnixBytesV1,
    #[cfg(not(unix))]
    Utf8V1,
}

impl RelativePathIdentity {
    fn from_path(root: &Path, path: &Path) -> Result<Self, RunnerError> {
        let relative = path.strip_prefix(root).map_err(|_| {
            RunnerError::InvalidCase(format!(
                "path {} is outside root {}",
                path.display(),
                root.display()
            ))
        })?;
        let mut bytes = Vec::new();
        for component in relative.components() {
            let Component::Normal(component) = component else {
                if matches!(component, Component::CurDir) {
                    continue;
                }
                return Err(RunnerError::InvalidCase(format!(
                    "path identity is not relative: {}",
                    path.display()
                )));
            };
            if !bytes.is_empty() {
                bytes.push(b'/');
            }
            append_component_bytes(&mut bytes, component)?;
        }
        Ok(Self {
            encoding: path_identity_encoding(),
            bytes,
        })
    }

    fn from_text(value: &str) -> Self {
        Self {
            encoding: path_identity_encoding(),
            bytes: value.as_bytes().to_vec(),
        }
    }

    pub fn from_raw_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, RunnerError> {
        let bytes = bytes.into();
        validate_identity_bytes(&bytes)?;
        #[cfg(not(unix))]
        if std::str::from_utf8(&bytes).is_err() {
            return Err(RunnerError::InvalidCase(
                "non-UTF-8 path identity is unsupported on this platform".to_owned(),
            ));
        }
        Ok(Self {
            encoding: path_identity_encoding(),
            bytes,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn encoding(&self) -> &'static str {
        match self.encoding {
            PathIdentityEncoding::UnixBytesV1 => "unix-bytes-v1",
            #[cfg(not(unix))]
            PathIdentityEncoding::Utf8V1 => "utf8-v1",
        }
    }

    /// Stable lower-case escaped display, suitable for diagnostics and JSON.
    pub fn display(&self) -> String {
        escaped_path_bytes(&self.bytes)
    }

    fn to_path(&self, root: &Path) -> Result<PathBuf, RunnerError> {
        if self.bytes.is_empty() {
            return Ok(root.to_path_buf());
        }
        let mut path = root.to_path_buf();
        for component in self.bytes.split(|byte| *byte == b'/') {
            if component.is_empty() {
                return Err(RunnerError::InvalidCase(
                    "path identity contains an empty component".to_owned(),
                ));
            }
            #[cfg(unix)]
            path.push(OsString::from_vec(component.to_vec()));
            #[cfg(not(unix))]
            path.push(std::str::from_utf8(component).map_err(|_| {
                RunnerError::InvalidCase(
                    "non-UTF-8 path identity is unsupported on this platform".to_owned(),
                )
            })?);
        }
        Ok(path)
    }
}

fn path_identity_encoding() -> PathIdentityEncoding {
    #[cfg(unix)]
    {
        PathIdentityEncoding::UnixBytesV1
    }
    #[cfg(not(unix))]
    {
        PathIdentityEncoding::Utf8V1
    }
}

fn append_component_bytes(output: &mut Vec<u8>, component: &OsStr) -> Result<(), RunnerError> {
    #[cfg(unix)]
    {
        output.extend_from_slice(component.as_bytes());
        Ok(())
    }
    #[cfg(not(unix))]
    {
        output.extend_from_slice(
            component
                .to_str()
                .ok_or_else(|| {
                    RunnerError::InvalidCase(
                        "non-UTF-8 path identity is unsupported on this platform".to_owned(),
                    )
                })?
                .as_bytes(),
        );
        Ok(())
    }
}

fn validate_identity_bytes(bytes: &[u8]) -> Result<(), RunnerError> {
    if bytes.contains(&0)
        || bytes
            .split(|byte| *byte == b'/')
            .any(|part| part == b"." || part == b".." || part.is_empty())
    {
        return Err(RunnerError::InvalidCase(format!(
            "path identity is not a safe relative path: {}",
            escaped_path_bytes(bytes)
        )));
    }
    Ok(())
}

fn escaped_path_bytes(bytes: &[u8]) -> String {
    let mut output = String::new();
    for byte in bytes {
        match *byte {
            b'/' => output.push('/'),
            b' '..=b'~' if *byte != b'\\' => output.push(*byte as char),
            byte => output.push_str(&format!("\\x{byte:02x}")),
        }
    }
    output
}

impl FixtureBinarySpec {
    pub fn shell(name: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: FixtureBinarySource::ShellScript(script.into()),
        }
    }

    pub fn existing(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            source: FixtureBinarySource::Existing(path.into()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> &FixtureBinarySource {
        &self.source
    }
}

/// One exact expected file and its bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedFile {
    relative_path: String,
    identity: RelativePathIdentity,
    contents: Option<Vec<u8>>,
}

impl ExpectedFile {
    pub fn path(relative_path: impl Into<String>) -> Self {
        let relative_path = relative_path.into();
        Self {
            identity: RelativePathIdentity::from_text(&relative_path),
            relative_path,
            contents: None,
        }
    }

    pub fn with_contents(relative_path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        let relative_path = relative_path.into();
        Self {
            identity: RelativePathIdentity::from_text(&relative_path),
            relative_path,
            contents: Some(contents.into()),
        }
    }

    /// Add an expected file whose name is represented by raw Unix bytes.
    /// This is the only way to assert a non-UTF-8 filename without losing its
    /// identity in a diagnostic conversion.
    pub fn raw_path(relative_path: impl Into<Vec<u8>>) -> Result<Self, RunnerError> {
        let identity = RelativePathIdentity::from_raw_bytes(relative_path)?;
        Ok(Self {
            relative_path: identity.display(),
            identity,
            contents: None,
        })
    }

    pub fn raw_path_with_contents(
        relative_path: impl Into<Vec<u8>>,
        contents: impl Into<Vec<u8>>,
    ) -> Result<Self, RunnerError> {
        let identity = RelativePathIdentity::from_raw_bytes(relative_path)?;
        Ok(Self {
            relative_path: identity.display(),
            identity,
            contents: Some(contents.into()),
        })
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn contents(&self) -> Option<&[u8]> {
        self.contents.as_deref()
    }

    pub fn identity(&self) -> &RelativePathIdentity {
        &self.identity
    }
}

/// One Git fixture authority whose refs are snapshotted before and after the
/// child process.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GitRoot {
    Source,
    Destination,
    Remote,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ExpectedGitChanges {
    source: BTreeMap<String, Option<String>>,
    destination: BTreeMap<String, Option<String>>,
    remote: BTreeMap<String, Option<String>>,
    operations: BTreeMap<GitRoot, ExpectedGitOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExpectedGitOperation {
    RefOnly,
    CommitDelivery {
        reference: String,
        tip: String,
        allow_index: bool,
        allow_message: bool,
    },
}

impl ExpectedGitChanges {
    fn set(&mut self, root: GitRoot, reference: String, value: Option<String>) {
        let refs = match root {
            GitRoot::Source => &mut self.source,
            GitRoot::Destination => &mut self.destination,
            GitRoot::Remote => &mut self.remote,
        };
        refs.insert(reference, value);
        self.operations
            .entry(root)
            .or_insert(ExpectedGitOperation::RefOnly);
    }

    fn for_root(&self, root: GitRoot) -> &BTreeMap<String, Option<String>> {
        match root {
            GitRoot::Source => &self.source,
            GitRoot::Destination => &self.destination,
            GitRoot::Remote => &self.remote,
        }
    }

    fn operation(&self, root: GitRoot) -> Option<&ExpectedGitOperation> {
        self.operations.get(&root)
    }
}

/// Assertions supplied by the owning journey definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedEffects {
    effect_root: String,
    exit_code: Option<i32>,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
    exact_files: Option<Vec<ExpectedFile>>,
    required_files: Vec<ExpectedFile>,
    forbidden_files: Vec<String>,
    output_limit: usize,
    timeout: Duration,
    redaction_secrets: Vec<String>,
    git_changes: ExpectedGitChanges,
}

impl Default for ExpectedEffects {
    fn default() -> Self {
        Self {
            effect_root: "effects".to_owned(),
            exit_code: None,
            stdout: None,
            stderr: None,
            exact_files: None,
            required_files: Vec::new(),
            forbidden_files: Vec::new(),
            output_limit: MAX_EVIDENCE_BYTES,
            timeout: DEFAULT_TIMEOUT,
            redaction_secrets: Vec::new(),
            git_changes: ExpectedGitChanges::default(),
        }
    }
}

impl ExpectedEffects {
    pub fn success() -> Self {
        Self {
            exit_code: Some(0),
            ..Self::default()
        }
    }

    pub fn effect_root(mut self, relative_path: impl Into<String>) -> Self {
        self.effect_root = relative_path.into();
        self
    }

    pub fn exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }

    pub fn stdout(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdout = Some(bytes.into());
        self
    }

    pub fn stderr(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stderr = Some(bytes.into());
        self
    }

    /// Require an exact file set below the effect root.  Extra files fail the
    /// case, which keeps journey assertions fail-closed.
    pub fn exact_files(mut self, files: impl IntoIterator<Item = ExpectedFile>) -> Self {
        self.exact_files = Some(files.into_iter().collect());
        self
    }

    pub fn require_file(mut self, file: ExpectedFile) -> Self {
        self.required_files.push(file);
        self
    }

    pub fn forbid_file(mut self, relative_path: impl Into<String>) -> Self {
        self.forbidden_files.push(relative_path.into());
        self
    }

    pub fn output_limit(mut self, limit: usize) -> Self {
        self.output_limit = limit;
        self
    }

    /// Set a finite process timeout for this case.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add a known fixture secret to the canonical diagnostic redactor.
    pub fn redact_secret(mut self, secret: impl Into<String>) -> Self {
        self.redaction_secrets.push(secret.into());
        self
    }

    /// Assert one exact local or remote Git ref delta.
    pub fn expect_git_ref(
        mut self,
        root: GitRoot,
        reference: impl Into<String>,
        value: Option<String>,
    ) -> Self {
        self.git_changes.set(root, reference.into(), value);
        self
    }

    /// Declare a complete Git commit-delivery effect.  Ref-only expectations
    /// remain the default and never authorize administrative Git files.
    pub fn expect_git_commit_delivery(
        mut self,
        root: GitRoot,
        reference: impl Into<String>,
        tip: impl Into<String>,
        allow_index: bool,
        allow_message: bool,
    ) -> Self {
        let reference = reference.into();
        let tip = tip.into();
        self.git_changes.operations.insert(
            root,
            ExpectedGitOperation::CommitDelivery {
                reference: reference.clone(),
                tip: tip.clone(),
                allow_index,
                allow_message,
            },
        );
        self.git_changes.set(root, reference, Some(tip));
        self
    }

    fn validate(&self) -> Result<(), RunnerError> {
        validate_relative_path(&self.effect_root)?;
        if self.output_limit < DIAGNOSTIC_TRUNCATION_MARKER.len()
            || self.output_limit > MAX_EVIDENCE_BYTES
        {
            return Err(RunnerError::InvalidCase(format!(
                "output limit must be between {} and canonical MAX_EVIDENCE_BYTES ({MAX_EVIDENCE_BYTES})",
                DIAGNOSTIC_TRUNCATION_MARKER.len()
            )));
        }
        if self.timeout.is_zero() {
            return Err(RunnerError::InvalidCase(
                "runner timeout must be finite and greater than zero".to_owned(),
            ));
        }
        if let Some(files) = &self.exact_files {
            validate_expected_files(files)?;
        }
        validate_expected_files(&self.required_files)?;
        for path in &self.forbidden_files {
            validate_relative_path(path)?;
        }
        Ok(())
    }
}

/// Inputs to one executable case.  Journey behavior is not encoded here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerCase {
    id: CaseId,
    seed: u64,
    binary: FixtureBinarySpec,
    args: Vec<String>,
    expected: ExpectedEffects,
}

impl RunnerCase {
    pub fn new(id: impl AsRef<str>, binary: FixtureBinarySpec) -> Result<Self, RunnerError> {
        Ok(Self {
            id: CaseId::parse(id.as_ref())?,
            seed: 0,
            binary,
            args: Vec::new(),
            expected: ExpectedEffects::default(),
        })
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Result<Self, RunnerError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        if self.args.iter().any(|arg| arg.as_bytes().contains(&0)) {
            return Err(RunnerError::InvalidCase("argument contains NUL".to_owned()));
        }
        Ok(self)
    }

    pub fn expected(mut self, expected: ExpectedEffects) -> Self {
        self.expected = expected;
        self
    }

    pub fn id(&self) -> &CaseId {
        &self.id
    }

    pub fn seed_value(&self) -> u64 {
        self.seed
    }

    pub fn binary(&self) -> &FixtureBinarySpec {
        &self.binary
    }

    pub fn arguments(&self) -> &[String] {
        &self.args
    }

    pub fn expectations(&self) -> &ExpectedEffects {
        &self.expected
    }
}

/// Exit status and bounded output captured from a fixture process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessCapture {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub spawn_error: Option<String>,
    pub timed_out: bool,
    pub tree_terminated: bool,
    pub reaped: bool,
    pub descendants_detected: bool,
    pub termination_error: Option<String>,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

impl ProcessCapture {
    pub fn success(&self) -> bool {
        self.code == Some(0) && self.signal.is_none()
    }
}

/// One bounded output stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedOutput {
    pub bytes: Vec<u8>,
    pub truncated: bool,
    pub redacted: bool,
    pub control_escaped: bool,
    pub non_utf8: bool,
}

/// Metadata for a regular file found below the fixture root.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ArtifactMetadata {
    pub relative_path: String,
    pub identity: RelativePathIdentity,
    pub size: u64,
    pub fingerprint: String,
}

/// A bounded filesystem identity record. File contents are represented by a
/// fixed-size fingerprint, never an unbounded byte buffer.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FilesystemEntry {
    pub relative_path: String,
    pub identity: RelativePathIdentity,
    pub kind: FilesystemEntryKind,
    pub size: u64,
    pub fingerprint: Option<String>,
    pub device: Option<u64>,
    pub inode: Option<u64>,
    pub nlink: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FilesystemEntryKind {
    Directory,
    Regular,
    Symlink,
    NonRegular,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesystemSnapshot {
    pub root: PathBuf,
    pub exists: bool,
    pub entries: Vec<FilesystemEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutsideCanarySnapshot {
    pub root: PathBuf,
    pub exists: bool,
    pub entries: Vec<FilesystemEntry>,
}

/// The runner's containment proof. The process receives only paths rooted
/// below `fixture_root` plus a controlled outside canary. Operating systems do
/// not provide a portable way to observe every arbitrary write in the world;
/// the hermetic environment instead makes writes to known outside boundaries
/// unavailable or detectable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainmentProof {
    pub fixture_root: PathBuf,
    pub checked_paths: Vec<PathBuf>,
    pub outside_paths: Vec<PathBuf>,
    pub unauthorized_paths: Vec<PathBuf>,
    pub nonregular_paths: Vec<PathBuf>,
    pub before: Vec<FilesystemSnapshot>,
    pub after: Vec<FilesystemSnapshot>,
    pub outside_before: OutsideCanarySnapshot,
    pub outside_after: OutsideCanarySnapshot,
    pub hard_link_paths: Vec<PathBuf>,
}

impl ContainmentProof {
    pub fn no_outside_writes(&self) -> bool {
        self.outside_paths.is_empty()
            && self.unauthorized_paths.is_empty()
            && self.nonregular_paths.is_empty()
            && self.hard_link_paths.is_empty()
            && self
                .checked_paths
                .iter()
                .all(|path| path.starts_with(&self.fixture_root))
    }
}

/// A completed case and its evidence bundle.  On success the temporary root
/// is removed after the in-memory evidence is assembled.  On failed cases the
/// root is retained so the selected bundle can be inspected and replayed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReport {
    pub contract_version: String,
    pub case_id: String,
    pub replay_id: String,
    pub root: PathBuf,
    pub artifact_root: PathBuf,
    pub effect_root: PathBuf,
    pub evidence_relative_path: String,
    pub evidence_json: String,
    pub evidence_bundle: EvidenceBundle,
    pub process: ProcessCapture,
    pub binary: ArtifactMetadata,
    pub artifacts: Vec<ArtifactMetadata>,
    pub containment: ContainmentProof,
    pub cleanup: CleanupReport,
    pub git: GitFixtureMetadata,
}

impl RunReport {
    /// Render the small terminal projection.  Detailed streams and artifact
    /// records stay in the report and evidence bundle.
    pub fn concise_status(&self) -> String {
        let outcome = if self.process.success() {
            "passed"
        } else {
            "failed"
        };
        format!(
            "e2e {case}: {outcome} replay={replay} artifacts={artifacts} evidence={evidence}",
            case = self.case_id,
            replay = self.replay_id,
            artifacts = self.artifacts.len(),
            evidence = self.evidence_relative_path,
        )
    }
}

/// Local Git fixture paths created for every case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFixtureMetadata {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub remote: PathBuf,
    pub source_refs: GitRefState,
    pub destination_refs: GitRefState,
    pub remote_refs: GitRefState,
    pub source_before: GitRefState,
    pub destination_before: GitRefState,
    pub remote_before: GitRefState,
    pub source_after: GitRefState,
    pub destination_after: GitRefState,
    pub remote_after: GitRefState,
    pub source_admin_before: GitAdministrativeSnapshot,
    pub destination_admin_before: GitAdministrativeSnapshot,
    pub remote_admin_before: GitAdministrativeSnapshot,
    pub source_admin_after: GitAdministrativeSnapshot,
    pub destination_admin_after: GitAdministrativeSnapshot,
    pub remote_admin_after: GitAdministrativeSnapshot,
    pub administrative_violations: Vec<GitAdministrativeViolation>,
    pub unexpected_changes: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitRefState {
    pub head: Option<String>,
    pub symbolic_head: Option<String>,
    pub refs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitAdministrativeSnapshot {
    pub root: PathBuf,
    pub entries: Vec<FilesystemEntry>,
    pub object_ids: BTreeSet<String>,
    pub reachable_object_ids: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GitViolationCategory {
    Config,
    Hook,
    Index,
    CommitMessage,
    Object,
    PackedRefs,
    UnrelatedRef,
    UnknownAdmin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitAdministrativeViolation {
    pub root: GitRoot,
    pub category: GitViolationCategory,
    pub path: RelativePathIdentity,
}

/// Runner entry point.
#[derive(Debug, Default, Clone, Copy)]
pub struct E2eRunner;

impl E2eRunner {
    pub fn new() -> Self {
        Self
    }

    /// Run one named case in a fresh fixture root.
    pub fn run(&self, case: RunnerCase) -> Result<RunReport, RunnerError> {
        ensure_supported_platform()?;
        case.expected.validate()?;
        validate_binary_spec(&case.binary)?;

        let replay_id = replay_id(&case);
        let mut fixture = LifecycleFixture::create(FixtureSpec::new(case.id.as_str(), case.seed))?;
        let outside_canary = create_outside_canary()?;
        let root = fixture.roots().root().to_path_buf();
        let result = self.run_in_fixture(&mut fixture, &case, &replay_id, &root, &outside_canary);
        match result {
            Ok(mut report) => {
                report.cleanup = fixture.cleanup(FixtureOutcome::Success);
                Ok(report)
            }
            Err(RunnerError::ExpectationFailed {
                details,
                mut report,
            }) => {
                report.cleanup = fixture.cleanup(FixtureOutcome::Failure);
                Err(RunnerError::ExpectationFailed { details, report })
            }
            Err(RunnerError::Timeout { root, mut report }) => {
                report.cleanup = fixture.cleanup(FixtureOutcome::Failure);
                Err(RunnerError::Timeout { root, report })
            }
            Err(RunnerError::Spawn {
                binary,
                source,
                root,
                report: Some(mut report),
            }) => {
                report.cleanup = fixture.cleanup(FixtureOutcome::Failure);
                Err(RunnerError::Spawn {
                    binary,
                    source,
                    root,
                    report: Some(report),
                })
            }
            Err(error) => {
                // Errors before evidence assembly do not have a selected
                // bundle to retain. Dropping the TempDir removes the root.
                drop(fixture);
                Err(error)
            }
        }
    }

    fn run_in_fixture(
        &self,
        fixture: &mut LifecycleFixture,
        case: &RunnerCase,
        replay: &str,
        root: &Path,
        outside_canary: &TempDir,
    ) -> Result<RunReport, RunnerError> {
        let evidence_file = format!("{}.jsonl", case.id.as_str());
        let evidence_relative_path = format!("artifacts/{evidence_file}");
        let artifact = ArtifactReference::new(evidence_relative_path.clone(), replay)?;
        let source_plan = SourcePlanConfig::new("fixture", replay, "runner")?;
        let identity = TestIdentity::new(
            case.id.as_str(),
            "omnirepo-e2e",
            "fixture-root",
            "process",
            source_plan,
            1,
            case.seed,
            "black-box-e2e",
        )?;
        let recorder = EventRecorder::new(DiagnosticRedactor::new(
            case.expected.redaction_secrets.iter().cloned().chain([
                root.display().to_string(),
                outside_canary.path().display().to_string(),
            ]),
        ));
        let step = recorder.start(identity, artifact)?;
        let result = self.run_in_fixture_work(
            fixture,
            case,
            replay,
            root,
            evidence_relative_path,
            outside_canary,
        );
        match result {
            Ok(mut pending) => {
                let outcome = if pending.failure_details.is_some() {
                    Outcome::Failed
                } else {
                    Outcome::Passed
                };
                step.finish(outcome, pending.failure_details.as_deref())?;
                let bundle = recorder.finalize()?;
                let evidence_json = bundle.to_jsonl()?;
                let store = ArtifactStore::new(fixture.roots().artifacts())?;
                store.write_bundle(&evidence_file, &bundle)?;
                fixture.record(
                    "e2e.evidence.write",
                    format!(
                        "path={};bytes={};schema={}",
                        pending.evidence_relative_path,
                        evidence_json.len(),
                        bundle.schema
                    ),
                );
                let failure_details = pending.failure_details.take();
                let timed_out = pending.process.timed_out;
                let spawn_error = pending.process.spawn_error.clone();
                let binary_path = pending.binary_path.clone();
                let report = RunReport {
                    contract_version: E2E_RUNNER_CONTRACT_VERSION.to_owned(),
                    case_id: pending.case_id,
                    replay_id: pending.replay_id,
                    root: pending.root,
                    artifact_root: pending.artifact_root,
                    effect_root: pending.effect_root,
                    evidence_relative_path: pending.evidence_relative_path,
                    evidence_json,
                    evidence_bundle: bundle,
                    process: pending.process,
                    binary: pending.binary,
                    artifacts: pending.artifacts,
                    containment: pending.containment,
                    cleanup: pending.cleanup,
                    git: pending.git,
                };
                if timed_out {
                    return Err(RunnerError::Timeout {
                        root: report.root.clone(),
                        report: Box::new(report),
                    });
                }
                if let Some(spawn_error) = spawn_error.as_deref() {
                    return Err(RunnerError::Spawn {
                        binary: binary_path,
                        source: io::Error::other(spawn_error),
                        root: report.root.clone(),
                        report: Some(Box::new(report)),
                    });
                }
                if let Some(details) = failure_details {
                    return Err(RunnerError::ExpectationFailed {
                        details,
                        report: Box::new(report),
                    });
                }
                Ok(report)
            }
            Err(error) => {
                let diagnostic = error.to_string();
                let _ = step.harness_failure(&diagnostic);
                if let Ok(bundle) = recorder.finalize() {
                    if let Ok(store) = ArtifactStore::new(fixture.roots().artifacts()) {
                        let _ = store.write_bundle(&evidence_file, &bundle);
                    }
                }
                Err(error)
            }
        }
    }

    fn run_in_fixture_work(
        &self,
        fixture: &mut LifecycleFixture,
        case: &RunnerCase,
        replay: &str,
        root: &Path,
        evidence_relative_path: String,
        outside_canary: &TempDir,
    ) -> Result<PendingRun, RunnerError> {
        let git = provision_git_fixtures(fixture)?;
        let effects = fixture
            .roots()
            .resolve(RootKind::Root, &case.expected.effect_root)?;
        fs::create_dir_all(&effects)?;

        let binary_path = build_binary(fixture, &case.binary)?;
        let binary = metadata_for_path(fixture, &binary_path)?;
        let checked_paths = process_paths(fixture, replay, &effects);
        let before = snapshot_authorized_roots(fixture)?;
        let outside_before = snapshot_outside_canary(outside_canary)?;
        let containment = ContainmentProof {
            fixture_root: root.to_path_buf(),
            checked_paths,
            outside_paths: Vec::new(),
            unauthorized_paths: Vec::new(),
            nonregular_paths: Vec::new(),
            before,
            after: Vec::new(),
            outside_before,
            outside_after: OutsideCanarySnapshot {
                root: outside_canary.path().to_path_buf(),
                exists: true,
                entries: Vec::new(),
            },
            hard_link_paths: Vec::new(),
        };

        ensure_no_preexisting_hard_links(&containment.before[0], &containment.outside_before)?;

        fixture.record(
            "e2e.case.select",
            format!(
                "case={};replay={replay};binary={}",
                case.id.as_str(),
                binary.relative_path
            ),
        );
        let process = invoke_binary(fixture, case, replay, &binary_path, root, outside_canary)?;
        let after = snapshot_authorized_roots(fixture)?;
        let outside_after = snapshot_outside_canary(outside_canary)?;
        let mut git = git;
        git.source_after = capture_git_refs(fixture, &git.source)?;
        git.destination_after = capture_git_refs(fixture, &git.destination)?;
        git.remote_after = capture_git_refs(fixture, &git.remote)?;
        git.source_admin_after = capture_git_admin(fixture, &git.source, GitRoot::Source)?;
        git.destination_admin_after =
            capture_git_admin(fixture, &git.destination, GitRoot::Destination)?;
        git.remote_admin_after = capture_git_admin(fixture, &git.remote, GitRoot::Remote)?;
        git.source_refs = git.source_after.clone();
        git.destination_refs = git.destination_after.clone();
        git.remote_refs = git.remote_after.clone();
        write_process_artifacts(fixture, case, &process)?;
        let artifacts = collect_artifacts(fixture, root)?;
        let mut containment = containment;
        containment.after = after;
        containment.outside_after = outside_after;
        let (unauthorized, nonregular) = compare_snapshots(
            &containment.before,
            &containment.after,
            &effects,
            &fixture.roots().artifacts().join("profiles"),
            &git,
            &case.expected.git_changes,
        );
        containment.unauthorized_paths = unauthorized;
        containment.nonregular_paths = nonregular;
        containment.outside_paths =
            compare_outside_snapshots(&containment.outside_before, &containment.outside_after);
        containment.hard_link_paths = compare_hard_link_identities(
            &containment.before[0],
            &containment.after[0],
            &containment.outside_before,
            &containment.outside_after,
        );
        if !containment.outside_paths.is_empty() {
            let store = ArtifactStore::new(fixture.roots().artifacts())?;
            let details = containment
                .outside_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            store.write_bytes(
                Path::new(case.id.as_str()).join("outside-canary.txt"),
                details.as_bytes(),
            )?;
        }
        containment.checked_paths.extend(
            artifacts
                .iter()
                .map(|artifact| artifact.identity.to_path(root))
                .collect::<Result<Vec<_>, _>>()?,
        );

        let expectation = check_expectations(fixture, case, &process, &effects);
        let mut failure_details = expectation.err();
        if process.spawn_error.is_some() {
            failure_details = Some(format!(
                "spawn failed: {}",
                process
                    .spawn_error
                    .as_deref()
                    .unwrap_or("unknown spawn error")
            ));
        } else if process.timed_out {
            failure_details =
                Some("process timed out and its process group was terminated".to_owned());
        }
        if !containment.unauthorized_paths.is_empty()
            || !containment.outside_paths.is_empty()
            || !containment.nonregular_paths.is_empty()
            || !containment.hard_link_paths.is_empty()
        {
            let mut details = String::from("fixture containment violation");
            if !containment.unauthorized_paths.is_empty() {
                details.push_str(&format!(
                    "; unauthorized={:?}",
                    containment.unauthorized_paths
                ));
            }
            if !containment.outside_paths.is_empty() {
                details.push_str(&format!("; outside={:?}", containment.outside_paths));
            }
            if !containment.nonregular_paths.is_empty() {
                details.push_str(&format!("; nonregular={:?}", containment.nonregular_paths));
            }
            if !containment.hard_link_paths.is_empty() {
                details.push_str(&format!("; hard_links={:?}", containment.hard_link_paths));
            }
            failure_details = Some(details);
        }
        git.administrative_violations =
            git_administrative_violations(&git, &case.expected.git_changes);
        git.unexpected_changes = git_ref_changes_unexpected(&git, &case.expected.git_changes)
            || !git.administrative_violations.is_empty();
        if git.unexpected_changes {
            failure_details = Some(format!(
                "unexpected Git administrative change: {:?}",
                git.administrative_violations
            ));
        }
        Ok(PendingRun {
            case_id: case.id.as_str().to_owned(),
            replay_id: replay.to_owned(),
            root: root.to_path_buf(),
            artifact_root: fixture.roots().artifacts().to_path_buf(),
            effect_root: effects.to_path_buf(),
            evidence_relative_path,
            process,
            binary_path,
            binary,
            artifacts,
            containment,
            cleanup: CleanupReport {
                root: root.to_path_buf(),
                removed: false,
                retained: false,
                expected_residue: Vec::new(),
                leaks: Vec::new(),
            },
            git,
            failure_details,
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_supported_platform() -> Result<(), RunnerError> {
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn ensure_supported_platform() -> Result<(), RunnerError> {
    Err(RunnerError::InvalidCase(
        "the E2E runner supports only Linux and macOS process-tree controls".to_owned(),
    ))
}

struct PendingRun {
    case_id: String,
    replay_id: String,
    root: PathBuf,
    artifact_root: PathBuf,
    effect_root: PathBuf,
    evidence_relative_path: String,
    process: ProcessCapture,
    binary_path: PathBuf,
    binary: ArtifactMetadata,
    artifacts: Vec<ArtifactMetadata>,
    containment: ContainmentProof,
    cleanup: CleanupReport,
    git: GitFixtureMetadata,
    failure_details: Option<String>,
}

fn validate_binary_spec(spec: &FixtureBinarySpec) -> Result<(), RunnerError> {
    validate_single_component(&spec.name, "fixture binary name")?;
    match &spec.source {
        FixtureBinarySource::ShellScript(script) if script.is_empty() => Err(RunnerError::Build {
            binary: spec.name.clone(),
            reason: "shell fixture script is empty".to_owned(),
        }),
        FixtureBinarySource::ShellScript(script) if !script.as_bytes().starts_with(b"#!") => {
            Err(RunnerError::Build {
                binary: spec.name.clone(),
                reason: "shell fixture must begin with a shebang".to_owned(),
            })
        }
        FixtureBinarySource::ShellScript(_) => Ok(()),
        FixtureBinarySource::Existing(path) if !path.is_absolute() => Err(RunnerError::Build {
            binary: spec.name.clone(),
            reason: format!(
                "selected executable path is not absolute: {}",
                path.display()
            ),
        }),
        FixtureBinarySource::Existing(path) if !path.is_file() => Err(RunnerError::Build {
            binary: spec.name.clone(),
            reason: format!(
                "selected executable is not a regular file: {}",
                path.display()
            ),
        }),
        FixtureBinarySource::Existing(_) => Ok(()),
    }
}

fn validate_single_component(value: &str, field: &str) -> Result<(), RunnerError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\n')
        || value.contains('\r')
        || value.as_bytes().contains(&0)
    {
        return Err(RunnerError::InvalidCase(format!(
            "{field} must be one safe path component: {value:?}"
        )));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), RunnerError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || value.as_bytes().contains(&0)
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(RunnerError::InvalidCase(format!(
            "relative path is not confined: {value:?}"
        )));
    }
    Ok(())
}

fn validate_expected_files(files: &[ExpectedFile]) -> Result<(), RunnerError> {
    let mut paths = BTreeSet::new();
    for file in files {
        validate_relative_path(&file.relative_path)?;
        validate_identity_bytes(file.identity.bytes())?;
        if !paths.insert(file.identity.clone()) {
            return Err(RunnerError::InvalidCase(format!(
                "duplicate expected effect path: {:?}",
                file.relative_path
            )));
        }
    }
    Ok(())
}

fn replay_id(case: &RunnerCase) -> String {
    let mut hash = 0xcbf29ce484222325_u64 ^ case.seed;
    for byte in case
        .id
        .as_str()
        .bytes()
        .chain([0])
        .chain(case.binary.name.bytes())
        .chain([0])
        .chain(case.args.iter().flat_map(|arg| arg.bytes().chain([0])))
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{}-{hash:016x}", case.id.as_str())
}

fn build_binary(
    fixture: &mut LifecycleFixture,
    spec: &FixtureBinarySpec,
) -> Result<PathBuf, RunnerError> {
    let bytes = match &spec.source {
        FixtureBinarySource::ShellScript(script) => script.as_bytes().to_vec(),
        FixtureBinarySource::Existing(path) => {
            fs::read(path).map_err(|source| RunnerError::Build {
                binary: spec.name.clone(),
                reason: format!("read {}: {source}", path.display()),
            })?
        }
    };
    let path = fixture.publish_executable(&spec.name, &bytes)?;
    fixture.record(
        "e2e.binary.build",
        format!(
            "name={};source={:?};path={}",
            spec.name,
            spec.source,
            path.display()
        ),
    );
    Ok(path)
}

fn provision_git_fixtures(
    fixture: &mut LifecycleFixture,
) -> Result<GitFixtureMetadata, RunnerError> {
    let source_root = fixture.roots().source().to_path_buf();
    let destination_root = fixture.roots().destination().to_path_buf();
    let source = fixture
        .create_git_repository(source_root, DirtyGitState::Clean)?
        .root;
    let destination = fixture
        .create_git_repository(destination_root, DirtyGitState::Clean)?
        .root;
    let remote = fixture.roots().remote().to_path_buf();
    run_git(fixture, &remote, &["init", "--bare", "--quiet"])?;
    let remote_text = remote.to_str().ok_or_else(|| RunnerError::Build {
        binary: "git fixture".to_owned(),
        reason: "remote fixture path is not valid UTF-8".to_owned(),
    })?;
    run_git(
        fixture,
        &destination,
        &["remote", "add", "origin", remote_text],
    )?;
    fixture.record(
        "e2e.git.provision",
        format!(
            "source={};destination={};remote={}",
            relative_path(fixture.roots().root(), &source),
            relative_path(fixture.roots().root(), &destination),
            relative_path(fixture.roots().root(), &remote)
        ),
    );
    let source_refs = capture_git_refs(fixture, &source)?;
    let destination_refs = capture_git_refs(fixture, &destination)?;
    let remote_refs = capture_git_refs(fixture, &remote)?;
    let source_admin_before = capture_git_admin(fixture, &source, GitRoot::Source)?;
    let destination_admin_before = capture_git_admin(fixture, &destination, GitRoot::Destination)?;
    let remote_admin_before = capture_git_admin(fixture, &remote, GitRoot::Remote)?;
    Ok(GitFixtureMetadata {
        source,
        destination,
        remote,
        source_refs: source_refs.clone(),
        destination_refs: destination_refs.clone(),
        remote_refs: remote_refs.clone(),
        source_before: source_refs,
        destination_before: destination_refs,
        remote_before: remote_refs,
        source_after: GitRefState::default(),
        destination_after: GitRefState::default(),
        remote_after: GitRefState::default(),
        source_admin_before: source_admin_before.clone(),
        destination_admin_before: destination_admin_before.clone(),
        remote_admin_before: remote_admin_before.clone(),
        source_admin_after: source_admin_before,
        destination_admin_after: destination_admin_before,
        remote_admin_after: remote_admin_before,
        administrative_violations: Vec::new(),
        unexpected_changes: false,
    })
}

fn run_git(
    fixture: &LifecycleFixture,
    current_dir: &Path,
    args: &[&str],
) -> Result<(), RunnerError> {
    let mut command = Command::new("git");
    fixture.environment().apply(&mut command);
    command.current_dir(current_dir).args(args);
    let capture = run_command(
        command,
        DEFAULT_TIMEOUT,
        MAX_EVIDENCE_BYTES.min(64 * 1024),
        true,
        None,
    )
    .map_err(|error| RunnerError::Build {
        binary: "git fixture".to_owned(),
        reason: error.to_string(),
    })?;
    if capture.status.as_ref().is_some_and(ExitStatus::success) {
        Ok(())
    } else {
        let status = capture.status;
        Err(RunnerError::Build {
            binary: "git fixture".to_owned(),
            reason: format!(
                "git {} failed (code={:?}, signal={:?}): {}",
                args.join(" "),
                status.as_ref().and_then(ExitStatus::code),
                status.as_ref().and_then(signal),
                String::from_utf8_lossy(&capture.stdout.bytes)
            ),
        })
    }
}

fn capture_git_refs(
    fixture: &LifecycleFixture,
    repository: &Path,
) -> Result<GitRefState, RunnerError> {
    let mut command = Command::new("git");
    fixture.environment().apply(&mut command);
    command
        .current_dir(repository)
        .args(["for-each-ref", "--format=%(refname)=%(objectname)"]);
    let refs = run_command(command, DEFAULT_TIMEOUT, 64 * 1024, true, None).map_err(|error| {
        RunnerError::Build {
            binary: "git refs".to_owned(),
            reason: error.to_string(),
        }
    })?;
    if !refs.status.as_ref().is_some_and(ExitStatus::success) {
        return Err(RunnerError::Build {
            binary: "git refs".to_owned(),
            reason: format!(
                "ref listing failed (code={:?}, signal={:?})",
                refs.status.as_ref().and_then(ExitStatus::code),
                refs.status.as_ref().and_then(signal)
            ),
        });
    }
    let mut state = GitRefState::default();
    for line in String::from_utf8_lossy(&refs.stdout.bytes).lines() {
        let Some((reference, oid)) = line.split_once('=') else {
            return Err(RunnerError::Build {
                binary: "git refs".to_owned(),
                reason: format!("malformed ref listing line: {line:?}"),
            });
        };
        state.refs.insert(reference.to_owned(), oid.to_owned());
    }

    let mut head = Command::new("git");
    fixture.environment().apply(&mut head);
    head.current_dir(repository).args(["rev-parse", "HEAD"]);
    let head_capture = run_command(head, DEFAULT_TIMEOUT, 1024, true, None).map_err(|error| {
        RunnerError::Build {
            binary: "git HEAD".to_owned(),
            reason: error.to_string(),
        }
    })?;
    if head_capture
        .status
        .as_ref()
        .is_some_and(ExitStatus::success)
    {
        state.head = Some(
            String::from_utf8_lossy(&head_capture.stdout.bytes)
                .trim()
                .to_owned(),
        );
    }

    let mut symbolic = Command::new("git");
    fixture.environment().apply(&mut symbolic);
    symbolic
        .current_dir(repository)
        .args(["symbolic-ref", "-q", "HEAD"]);
    let symbolic_capture =
        run_command(symbolic, DEFAULT_TIMEOUT, 1024, true, None).map_err(|error| {
            RunnerError::Build {
                binary: "git symbolic HEAD".to_owned(),
                reason: error.to_string(),
            }
        })?;
    if symbolic_capture
        .status
        .as_ref()
        .is_some_and(ExitStatus::success)
    {
        state.symbolic_head = Some(
            String::from_utf8_lossy(&symbolic_capture.stdout.bytes)
                .trim()
                .to_owned(),
        );
    }
    Ok(state)
}

fn capture_git_admin(
    fixture: &LifecycleFixture,
    repository: &Path,
    root: GitRoot,
) -> Result<GitAdministrativeSnapshot, RunnerError> {
    let admin_root = if root == GitRoot::Remote {
        repository.to_path_buf()
    } else {
        repository.join(".git")
    };
    let snapshot = snapshot_tree(&admin_root, Some(fixture))?;
    let object_ids = snapshot
        .entries
        .iter()
        .filter_map(|entry| object_id_from_path(&entry.identity))
        .collect::<BTreeSet<_>>();
    let mut command = Command::new("git");
    fixture.environment().apply(&mut command);
    command
        .current_dir(repository)
        .args(["rev-list", "--objects", "--all"]);
    let reachable =
        run_command(command, DEFAULT_TIMEOUT, 256 * 1024, true, None).map_err(|error| {
            RunnerError::Build {
                binary: "git reachable objects".to_owned(),
                reason: error.to_string(),
            }
        })?;
    let reachable_object_ids = if reachable.status.as_ref().is_some_and(ExitStatus::success) {
        reachable
            .stdout
            .bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| line.split(|byte| *byte == b' ').next())
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter(|id| id.len() == 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(str::to_owned)
            .collect()
    } else {
        BTreeSet::new()
    };
    Ok(GitAdministrativeSnapshot {
        root: admin_root,
        entries: snapshot.entries,
        object_ids,
        reachable_object_ids,
    })
}

fn object_id_from_path(path: &RelativePathIdentity) -> Option<String> {
    let mut components = path.bytes().split(|byte| *byte == b'/');
    if components.next()? != b"objects" {
        return None;
    }
    let prefix = components.next()?;
    let suffix = components.next()?;
    if components.next().is_some()
        || prefix.len() != 2
        || suffix.len() != 38
        || !prefix.iter().chain(suffix).all(u8::is_ascii_hexdigit)
    {
        return None;
    }
    let prefix = std::str::from_utf8(prefix).ok()?;
    let suffix = std::str::from_utf8(suffix).ok()?;
    Some(format!("{prefix}{suffix}"))
}

fn process_paths(fixture: &LifecycleFixture, replay: &str, effects: &Path) -> Vec<PathBuf> {
    let roots = fixture.roots();
    vec![
        roots.root().to_path_buf(),
        roots.home().to_path_buf(),
        roots.machine_config().to_path_buf(),
        roots.source().to_path_buf(),
        roots.destination().to_path_buf(),
        roots.remote().to_path_buf(),
        roots.artifacts().to_path_buf(),
        effects.to_path_buf(),
        roots.home().join(format!(".omnirepo/runs/{replay}.log")),
    ]
}

fn create_outside_canary() -> Result<TempDir, RunnerError> {
    // This is a deliberate, controlled observation boundary. It is not a
    // claim that the host OS can globally report arbitrary writes.
    let canary = TempDir::new()?;
    fs::write(canary.path().join("sentinel"), b"outside-canary-v1\n")?;
    Ok(canary)
}

fn snapshot_authorized_roots(
    fixture: &LifecycleFixture,
) -> Result<Vec<FilesystemSnapshot>, RunnerError> {
    let roots = fixture.roots();
    let paths = [
        roots.root(),
        roots.home(),
        roots.machine_config_root(),
        roots.source(),
        roots.source_config_root(),
        roots.source_snapshot(),
        roots.destination(),
        roots.runs(),
        roots.artifacts(),
        roots.remote(),
    ];
    paths
        .into_iter()
        .map(|path| snapshot_tree(path, Some(fixture)))
        .collect()
}

fn snapshot_outside_canary(canary: &TempDir) -> Result<OutsideCanarySnapshot, RunnerError> {
    let snapshot = snapshot_tree(canary.path(), None)?;
    Ok(OutsideCanarySnapshot {
        root: snapshot.root,
        exists: snapshot.exists,
        entries: snapshot.entries,
    })
}

fn snapshot_tree(
    root: &Path,
    _fixture: Option<&LifecycleFixture>,
) -> Result<FilesystemSnapshot, RunnerError> {
    let root = root.to_path_buf();
    match fs::symlink_metadata(&root) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FilesystemSnapshot {
                root,
                exists: false,
                entries: Vec::new(),
            });
        }
        Err(error) => return Err(error.into()),
    }
    let mut entries = Vec::new();
    snapshot_entries(&root, &root, &mut entries)?;
    entries.sort();
    Ok(FilesystemSnapshot {
        root,
        exists: true,
        entries,
    })
}

fn snapshot_entries(
    root: &Path,
    current: &Path,
    entries: &mut Vec<FilesystemEntry>,
) -> Result<(), RunnerError> {
    let metadata = fs::symlink_metadata(current)?;
    entries.push(snapshot_entry(root, current, &metadata)?);
    if metadata.is_dir() {
        for child in fs::read_dir(current)? {
            let child = child?.path();
            snapshot_entries(root, &child, entries)?;
        }
    }
    Ok(())
}

fn snapshot_entry(
    root: &Path,
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<FilesystemEntry, RunnerError> {
    let kind = if metadata.file_type().is_symlink() {
        FilesystemEntryKind::Symlink
    } else if metadata.is_dir() {
        FilesystemEntryKind::Directory
    } else if metadata.is_file() {
        FilesystemEntryKind::Regular
    } else {
        FilesystemEntryKind::NonRegular
    };
    let fingerprint = if kind == FilesystemEntryKind::Regular {
        Some(fingerprint_file(path)?)
    } else if kind == FilesystemEntryKind::Symlink {
        Some(fs::read_link(path)?.display().to_string())
    } else {
        None
    };
    #[cfg(unix)]
    let (device, inode, nlink) = {
        use std::os::unix::fs::MetadataExt;
        (
            Some(metadata.dev()),
            Some(metadata.ino()),
            Some(metadata.nlink()),
        )
    };
    #[cfg(not(unix))]
    let (device, inode, nlink) = (None, None, None);
    let identity = RelativePathIdentity::from_path(root, path)?;
    Ok(FilesystemEntry {
        relative_path: identity.display(),
        identity,
        kind,
        size: metadata.len(),
        fingerprint,
        device,
        inode,
        nlink,
    })
}

fn fingerprint_file(path: &Path) -> Result<String, RunnerError> {
    let mut file = fs::File::open(path)?;
    let mut buffer = [0_u8; 8192];
    let mut hash = 0xcbf29ce484222325_u64;
    loop {
        let count = io::Read::read(&mut file, &mut buffer)?;
        if count == 0 {
            break;
        }
        for byte in &buffer[..count] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("fnv1a-{hash:016x}"))
}

fn regular_identity(entry: &FilesystemEntry) -> Option<(u64, u64)> {
    (entry.kind == FilesystemEntryKind::Regular).then_some((entry.device?, entry.inode?))
}

fn ensure_no_preexisting_hard_links(
    controlled: &FilesystemSnapshot,
    outside: &OutsideCanarySnapshot,
) -> Result<(), RunnerError> {
    let mut identities = HashMap::<(u64, u64), Vec<String>>::new();
    for (root, entries) in [
        (&controlled.root, controlled.entries.as_slice()),
        (&outside.root, outside.entries.as_slice()),
    ] {
        for entry in entries {
            if entry.kind != FilesystemEntryKind::Regular {
                continue;
            }
            let Some((device, inode)) = regular_identity(entry) else {
                return Err(RunnerError::InvalidCase(
                    "strict hard-link checks require Unix device/inode metadata".to_owned(),
                ));
            };
            let Some(nlink) = entry.nlink else {
                return Err(RunnerError::InvalidCase(
                    "strict hard-link checks require Unix link-count metadata".to_owned(),
                ));
            };
            let path = entry
                .identity
                .to_path(root)
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| entry.relative_path.clone());
            identities.entry((device, inode)).or_default().push(path);
            if nlink > 1 {
                return Err(RunnerError::InvalidCase(format!(
                    "preexisting hard link rejected before fixture spawn: {} (dev={device}, ino={inode}, nlink={nlink})",
                    entry.identity.display()
                )));
            }
        }
    }
    if let Some(((device, inode), paths)) =
        identities.into_iter().find(|(_, paths)| paths.len() > 1)
    {
        return Err(RunnerError::InvalidCase(format!(
            "preexisting hard-link identity rejected before fixture spawn: dev={device}, ino={inode}, paths={paths:?}"
        )));
    }
    Ok(())
}

fn compare_hard_link_identities(
    controlled_before: &FilesystemSnapshot,
    controlled_after: &FilesystemSnapshot,
    outside_before: &OutsideCanarySnapshot,
    outside_after: &OutsideCanarySnapshot,
) -> Vec<PathBuf> {
    let before = identity_groups(
        &controlled_before.root,
        &controlled_before.entries,
        &outside_before.root,
        &outside_before.entries,
    );
    let after = identity_groups(
        &controlled_after.root,
        &controlled_after.entries,
        &outside_after.root,
        &outside_after.entries,
    );
    let mut affected = BTreeSet::new();
    for (identity, paths) in &after {
        let before_paths = before.get(identity).cloned().unwrap_or_default();
        let current_hard_linked = paths.len() > 1 || paths.iter().any(|path| path.1 > 1);
        let changed = before_paths != *paths;
        if current_hard_linked && changed {
            affected.extend(paths.iter().map(|(path, _)| path.clone()));
            affected.extend(before_paths.iter().map(|(path, _)| path.clone()));
        }
    }
    affected.into_iter().collect()
}

type IdentityGroup = Vec<(PathBuf, u64)>;

fn identity_groups(
    controlled_root: &Path,
    controlled_entries: &[FilesystemEntry],
    outside_root: &Path,
    outside_entries: &[FilesystemEntry],
) -> HashMap<(u64, u64), IdentityGroup> {
    let mut groups = HashMap::new();
    for (root, entries) in [
        (controlled_root, controlled_entries),
        (outside_root, outside_entries),
    ] {
        for entry in entries {
            let Some(identity) = regular_identity(entry) else {
                continue;
            };
            let Ok(path) = entry.identity.to_path(root) else {
                continue;
            };
            groups
                .entry(identity)
                .or_insert_with(Vec::new)
                .push((path, entry.nlink.unwrap_or_default()));
        }
    }
    for paths in groups.values_mut() {
        paths.sort();
    }
    groups
}

fn compare_snapshots(
    before: &[FilesystemSnapshot],
    after: &[FilesystemSnapshot],
    effects: &Path,
    artifact_root: &Path,
    git: &GitFixtureMetadata,
    expected_git_changes: &ExpectedGitChanges,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut unauthorized = BTreeSet::new();
    let mut nonregular = BTreeSet::new();
    for (before, after) in before.iter().zip(after) {
        let before_map = before
            .entries
            .iter()
            .map(|entry| (entry.identity.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let after_map = after
            .entries
            .iter()
            .map(|entry| (entry.identity.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let mut paths = BTreeSet::new();
        paths.extend(before_map.keys().cloned());
        paths.extend(after_map.keys().cloned());
        for relative in paths {
            let previous = before_map.get(&relative);
            let current = after_map.get(&relative);
            if current.is_some_and(|entry| {
                matches!(
                    entry.kind,
                    FilesystemEntryKind::Symlink | FilesystemEntryKind::NonRegular
                )
            }) {
                if let Ok(path) = relative.to_path(&after.root) {
                    nonregular.insert(path);
                }
            }
            if previous == current && before.exists == after.exists {
                continue;
            }
            // Directory size changes are an implementation detail of creating
            // or removing children. Compare the stable identity instead so a
            // permitted effect does not make its parent look like an
            // unauthorized write. A replacement directory still changes its
            // inode/device and is therefore reported.
            if let (Some(previous), Some(current)) = (previous, current) {
                if previous.kind == FilesystemEntryKind::Directory
                    && current.kind == FilesystemEntryKind::Directory
                    && previous.fingerprint == current.fingerprint
                    && previous.device == current.device
                    && previous.inode == current.inode
                {
                    continue;
                }
            }
            let path = relative
                .to_path(&after.root)
                .unwrap_or_else(|_| after.root.clone());
            let allowed = path.starts_with(effects)
                || path.starts_with(artifact_root)
                || expected_git_write(&path, git, expected_git_changes);
            if !allowed {
                unauthorized.insert(path);
            }
        }
        if before.exists != after.exists && !after.root.starts_with(effects) {
            unauthorized.insert(after.root.clone());
        }
    }
    (
        unauthorized.into_iter().collect(),
        nonregular.into_iter().collect(),
    )
}

fn expected_git_write(
    path: &Path,
    git: &GitFixtureMetadata,
    expected: &ExpectedGitChanges,
) -> bool {
    [
        (GitRoot::Source, git.source.join(".git"), &expected.source),
        (
            GitRoot::Destination,
            git.destination.join(".git"),
            &expected.destination,
        ),
        (GitRoot::Remote, git.remote.clone(), &expected.remote),
    ]
    .into_iter()
    .find_map(|(root, admin_root, changes)| {
        let relative = path.strip_prefix(&admin_root).ok()?;
        let operation = expected.operation(root);
        Some(
            expected_git_relative_write(root, relative, changes, operation)
                || expected_git_object_write(root, relative, git, operation),
        )
    })
    .unwrap_or(false)
}

fn expected_git_object_write(
    root: GitRoot,
    relative: &Path,
    git: &GitFixtureMetadata,
    operation: Option<&ExpectedGitOperation>,
) -> bool {
    if !matches!(operation, Some(ExpectedGitOperation::CommitDelivery { .. })) {
        return false;
    }
    let identity = match relative_identity(relative) {
        Ok(identity) => identity,
        Err(_) => return false,
    };
    let Some(object_id) = object_id_from_path(&identity) else {
        return false;
    };
    let (before, after) = match root {
        GitRoot::Source => (&git.source_admin_before, &git.source_admin_after),
        GitRoot::Destination => (&git.destination_admin_before, &git.destination_admin_after),
        GitRoot::Remote => (&git.remote_admin_before, &git.remote_admin_after),
    };
    after.reachable_object_ids.contains(&object_id) && !before.object_ids.contains(&object_id)
}

fn expected_git_relative_write(
    _root: GitRoot,
    relative: &Path,
    changes: &BTreeMap<String, Option<String>>,
    operation: Option<&ExpectedGitOperation>,
) -> bool {
    if changes.is_empty() {
        return false;
    }
    let identity = match relative_identity(relative) {
        Ok(identity) => identity,
        Err(_) => return false,
    };
    let bytes = identity.bytes();
    let text = std::str::from_utf8(bytes).ok();
    let operation = operation.unwrap_or(&ExpectedGitOperation::RefOnly);
    match operation {
        ExpectedGitOperation::RefOnly => {
            text.is_some_and(|text| changes.keys().any(|change| expected_git_path(text, change)))
        }
        ExpectedGitOperation::CommitDelivery {
            reference,
            allow_index,
            allow_message,
            ..
        } => {
            let Some(text) = text else { return false };
            expected_git_path(text, reference)
                || text == "HEAD"
                || text == "logs/HEAD"
                || (text == "index" && *allow_index)
                || (text == "COMMIT_EDITMSG" && *allow_message)
        }
    }
}

fn expected_git_path(path: &str, declared: &str) -> bool {
    if declared == "HEAD" || declared == "HEAD@symbolic" {
        return path == "HEAD" || path == "logs/HEAD";
    }
    let declared = declared.strip_prefix("refs/").unwrap_or(declared);
    let reference = format!("refs/{declared}");
    path == reference || path == format!("logs/{reference}")
}

fn relative_identity(path: &Path) -> Result<RelativePathIdentity, RunnerError> {
    let mut bytes = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(RunnerError::InvalidCase(
                "Git administrative path is not relative".to_owned(),
            ));
        };
        if !bytes.is_empty() {
            bytes.push(b'/');
        }
        append_component_bytes(&mut bytes, component)?;
    }
    RelativePathIdentity::from_raw_bytes(bytes)
}

fn git_administrative_violations(
    git: &GitFixtureMetadata,
    expected: &ExpectedGitChanges,
) -> Vec<GitAdministrativeViolation> {
    [
        (
            GitRoot::Source,
            &git.source_admin_before,
            &git.source_admin_after,
            expected.operation(GitRoot::Source),
            expected.for_root(GitRoot::Source),
        ),
        (
            GitRoot::Destination,
            &git.destination_admin_before,
            &git.destination_admin_after,
            expected.operation(GitRoot::Destination),
            expected.for_root(GitRoot::Destination),
        ),
        (
            GitRoot::Remote,
            &git.remote_admin_before,
            &git.remote_admin_after,
            expected.operation(GitRoot::Remote),
            expected.for_root(GitRoot::Remote),
        ),
    ]
    .into_iter()
    .flat_map(|(root, before, after, operation, changes)| {
        git_admin_delta(root, before, after, operation, changes)
    })
    .collect()
}

fn git_admin_delta(
    root: GitRoot,
    before: &GitAdministrativeSnapshot,
    after: &GitAdministrativeSnapshot,
    operation: Option<&ExpectedGitOperation>,
    changes: &BTreeMap<String, Option<String>>,
) -> Vec<GitAdministrativeViolation> {
    let before_map = before
        .entries
        .iter()
        .map(|entry| (entry.identity.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let after_map = after
        .entries
        .iter()
        .map(|entry| (entry.identity.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut identities = BTreeSet::new();
    identities.extend(before_map.keys().cloned());
    identities.extend(after_map.keys().cloned());
    identities
        .into_iter()
        .filter_map(|identity| {
            let previous = before_map.get(&identity);
            let current = after_map.get(&identity);
            if previous == current {
                return None;
            }
            if let (Some(previous), Some(current)) = (previous, current) {
                if previous.kind == FilesystemEntryKind::Directory
                    && current.kind == FilesystemEntryKind::Directory
                    && previous.device == current.device
                    && previous.inode == current.inode
                {
                    return None;
                }
            }
            let path = current
                .or(previous)
                .and_then(|entry| entry.identity.to_path(&after.root).ok())?;
            let relative = path.strip_prefix(&after.root).ok()?;
            let object_id = relative_identity(relative)
                .ok()
                .and_then(|path| object_id_from_path(&path));
            if matches!(operation, Some(ExpectedGitOperation::CommitDelivery { .. }))
                && object_id.as_ref().is_some_and(|object_id| {
                    after.reachable_object_ids.contains(object_id)
                        && !before.object_ids.contains(object_id)
                })
            {
                return None;
            }
            if expected_git_relative_write(root, relative, changes, operation)
                && object_id.is_none()
            {
                return None;
            }
            let category = git_violation_category(relative, previous, current);
            Some(GitAdministrativeViolation {
                root,
                category,
                path: relative_identity(relative).ok()?,
            })
        })
        .collect()
}

fn git_violation_category(
    relative: &Path,
    _previous: Option<&&FilesystemEntry>,
    _current: Option<&&FilesystemEntry>,
) -> GitViolationCategory {
    let identity = relative_identity(relative).ok();
    let text = identity
        .as_ref()
        .and_then(|identity| std::str::from_utf8(identity.bytes()).ok())
        .unwrap_or_default();
    if text == "config" {
        GitViolationCategory::Config
    } else if text == "index" {
        GitViolationCategory::Index
    } else if text == "packed-refs" {
        GitViolationCategory::PackedRefs
    } else if text == "COMMIT_EDITMSG" {
        GitViolationCategory::CommitMessage
    } else if text.starts_with("hooks/") {
        GitViolationCategory::Hook
    } else if text.starts_with("objects/") {
        GitViolationCategory::Object
    } else if text.starts_with("refs/") || text.starts_with("logs/") || text == "HEAD" {
        GitViolationCategory::UnrelatedRef
    } else {
        GitViolationCategory::UnknownAdmin
    }
}

fn compare_outside_snapshots(
    before: &OutsideCanarySnapshot,
    after: &OutsideCanarySnapshot,
) -> Vec<PathBuf> {
    let before_map = before
        .entries
        .iter()
        .map(|entry| (entry.identity.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let after_map = after
        .entries
        .iter()
        .map(|entry| (entry.identity.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut paths = BTreeSet::new();
    paths.extend(before_map.keys().cloned());
    paths.extend(after_map.keys().cloned());
    paths
        .into_iter()
        .filter(|path| before_map.get(path) != after_map.get(path))
        .filter_map(|path| path.to_path(&after.root).ok())
        .collect()
}

fn invoke_binary(
    fixture: &LifecycleFixture,
    case: &RunnerCase,
    replay: &str,
    binary: &Path,
    root: &Path,
    outside_canary: &TempDir,
) -> Result<ProcessCapture, RunnerError> {
    let roots = fixture.roots();
    let effects = roots
        .resolve(RootKind::Root, &case.expected.effect_root)
        .map_err(RunnerError::Fixture)?;
    // Child profiles belong to the runner-owned artifact area, outside
    // destination and journey effect roots. The replay and PID placeholders
    // keep parallel runs and child processes distinct.
    let profile_root = roots.artifacts().join("profiles");
    fs::create_dir_all(&profile_root)?;
    let profile_path = profile_root.join(format!("{replay}-%p.profraw"));
    #[cfg(target_os = "linux")]
    let status_file = Some(tempfile::NamedTempFile::new()?);
    #[cfg(not(target_os = "linux"))]
    let status_file: Option<tempfile::NamedTempFile> = None;
    #[cfg(target_os = "linux")]
    let mut command = {
        let supervisor = supervisor_binary_path()?;
        let mut command = Command::new(supervisor);
        command
            .arg("--cwd")
            .arg(root)
            .arg("--target")
            .arg(binary)
            .arg("--")
            .args(&case.args);
        command
    };
    #[cfg(not(target_os = "linux"))]
    let mut command = {
        let mut command = Command::new(binary);
        command.args(&case.args);
        command
    };
    fixture.environment().apply(&mut command);
    command
        .current_dir(root)
        .env("OMNIREPO_E2E_CONTRACT", E2E_RUNNER_CONTRACT_VERSION)
        .env("OMNIREPO_E2E_CASE_ID", case.id.as_str())
        .env("OMNIREPO_E2E_REPLAY_ID", replay)
        .env("OMNIREPO_E2E_ROOT", root)
        .env("OMNIREPO_E2E_HOME", roots.home())
        .env("OMNIREPO_E2E_CONFIG", roots.machine_config())
        .env("OMNIREPO_E2E_SOURCE", roots.source())
        .env("OMNIREPO_E2E_DESTINATION", roots.destination())
        .env("OMNIREPO_E2E_REMOTE", roots.remote())
        .env("OMNIREPO_E2E_ARTIFACTS", roots.artifacts())
        .env("OMNIREPO_E2E_OUTSIDE_CANARY", outside_canary.path())
        .env("OMNIREPO_E2E_EFFECTS_ROOT", effects)
        .env("OMNIREPO_E2E_OFFLINE", "1")
        .env("OMNIREPO_E2E_NO_AMBIENT_CREDENTIALS", "1")
        .env("LLVM_PROFILE_FILE", profile_path)
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("TMPDIR", roots.artifacts())
        .env("TMP", roots.artifacts())
        .env("TEMP", roots.artifacts());
    #[cfg(target_os = "linux")]
    if let Some(status_file) = &status_file {
        command.env("OMNIREPO_E2E_SUPERVISOR_STATUS", status_file.path());
    }
    let capture = run_command(
        command,
        case.expected.timeout,
        // Keep one bounded sentinel byte beyond the canonical maximum. The
        // canonical channel helper then owns the final truncation marker even
        // when a stream is cut before it can be fully read.
        MAX_EVIDENCE_BYTES.saturating_add(1),
        true,
        status_file.as_ref().map(|file| file.path()),
    )?;
    let redactor =
        DiagnosticRedactor::new(case.expected.redaction_secrets.iter().cloned().chain([
            root.display().to_string(),
            outside_canary.path().display().to_string(),
        ]));
    let stdout_was_truncated = capture.stdout.truncated;
    let stderr_was_truncated = capture.stderr.truncated;
    let channels = sanitize_channels(
        &redactor,
        &capture.stdout.bytes,
        &capture.stderr.bytes,
        case.expected.output_limit,
    )?;
    let mut stdout = channels.stdout;
    let mut stderr = channels.stderr;
    stdout.truncated |= stdout_was_truncated;
    stderr.truncated |= stderr_was_truncated;
    Ok(ProcessCapture {
        code: capture
            .supervisor
            .as_ref()
            .and_then(|status| status.target_code)
            .or_else(|| capture.status.as_ref().and_then(ExitStatus::code)),
        signal: capture
            .supervisor
            .as_ref()
            .and_then(|status| status.target_signal)
            .or_else(|| capture.status.as_ref().and_then(signal)),
        spawn_error: capture
            .supervisor
            .as_ref()
            .and_then(|status| status.spawn_error.clone())
            .or(capture.spawn_error),
        timed_out: capture.timed_out,
        tree_terminated: capture
            .supervisor
            .as_ref()
            .map(|status| status.tree_terminated)
            .unwrap_or(capture.tree_terminated),
        reaped: capture
            .supervisor
            .as_ref()
            .map(|status| status.reaped)
            .unwrap_or(capture.reaped),
        descendants_detected: capture
            .supervisor
            .as_ref()
            .is_some_and(|status| status.descendants_detected),
        termination_error: capture.supervisor.as_ref().and_then(|status| {
            status
                .capability_failure
                .clone()
                .or_else(|| status.termination_error.clone())
        }),
        stdout: CapturedOutput {
            bytes: stdout.text.into_bytes(),
            truncated: stdout.truncated,
            redacted: stdout.redacted,
            control_escaped: stdout.control_escaped,
            non_utf8: stdout.non_utf8,
        },
        stderr: CapturedOutput {
            bytes: stderr.text.into_bytes(),
            truncated: stderr.truncated,
            redacted: stderr.redacted,
            control_escaped: stderr.control_escaped,
            non_utf8: stderr.non_utf8,
        },
    })
}

#[derive(Debug)]
struct CommandCapture {
    status: Option<ExitStatus>,
    stdout: RawOutput,
    stderr: RawOutput,
    spawn_error: Option<String>,
    timed_out: bool,
    tree_terminated: bool,
    reaped: bool,
    supervisor: Option<SupervisorStatus>,
}

#[derive(Clone, Debug, Default)]
struct SupervisorStatus {
    target_code: Option<i32>,
    target_signal: Option<i32>,
    spawn_error: Option<String>,
    tree_terminated: bool,
    reaped: bool,
    descendants_detected: bool,
    capability_failure: Option<String>,
    termination_error: Option<String>,
    survivor_count: usize,
}

#[derive(Debug, Default)]
struct RawOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

#[derive(Debug, Default)]
struct SharedOutput {
    stdout: RawOutput,
    stderr: RawOutput,
    limit: usize,
}

#[cfg(target_os = "linux")]
fn supervisor_binary_path() -> io::Result<PathBuf> {
    for variable in [
        "CARGO_BIN_EXE_omnirepo_e2e_supervisor",
        "CARGO_BIN_EXE_omnirepo-e2e-supervisor",
    ] {
        if let Some(path) = std::env::var_os(variable).map(PathBuf::from) {
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    let current = std::env::current_exe()?;
    let candidates = [
        current
            .parent()
            .and_then(Path::parent)
            .map(|path| path.join("omnirepo-e2e-supervisor")),
        current
            .parent()
            .map(|path| path.join("omnirepo-e2e-supervisor")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Cargo did not build omnirepo-e2e-supervisor for strict Linux E2E control",
            )
        })
}

fn run_command(
    mut command: Command,
    timeout: Duration,
    output_limit: usize,
    terminate_tree: bool,
    supervisor_status: Option<&Path>,
) -> io::Result<CommandCapture> {
    command
        .stdin(if supervisor_status.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(CommandCapture {
                status: None,
                stdout: RawOutput::default(),
                stderr: RawOutput::default(),
                spawn_error: Some(error.to_string()),
                timed_out: false,
                tree_terminated: false,
                reaped: false,
                supervisor: None,
            });
        }
    };
    let mut supervisor_stdin = if supervisor_status.is_some() {
        child.stdin.take()
    } else {
        None
    };
    let child_pid = child.id();
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = terminate_process_group(child_pid, true);
            let _ = child.wait();
            return Err(io::Error::other("child stdout pipe was unavailable"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = terminate_process_group(child_pid, true);
            let _ = child.wait();
            return Err(io::Error::other("child stderr pipe was unavailable"));
        }
    };
    let output = Arc::new(Mutex::new(SharedOutput {
        stdout: RawOutput::default(),
        stderr: RawOutput::default(),
        limit: output_limit,
    }));
    let stdout_join = spawn_reader(stdout, Arc::clone(&output), true);
    let stderr_join = spawn_reader(stderr, Arc::clone(&output), false);

    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let mut timed_out = false;
    let mut tree_terminated = false;
    let status = loop {
        let polled = match child.try_wait() {
            Ok(polled) => polled,
            Err(error) => {
                let _ = terminate_process_group(child_pid, true);
                let _ = child.wait();
                return Err(error);
            }
        };
        if let Some(status) = polled {
            if terminate_tree && process_group_exists(child_pid) {
                if let Err(error) = terminate_process_group(child_pid, false) {
                    let _ = child.wait();
                    return Err(error);
                }
            }
            tree_terminated = true;
            break Some(status);
        }
        if Instant::now() >= deadline {
            timed_out = true;
            if terminate_tree {
                if let Some(stdin) = supervisor_stdin.as_mut() {
                    let _ = io::Write::write_all(stdin, b"cancel\n");
                    let _ = io::Write::flush(stdin);
                    let grace_deadline = Instant::now() + PROCESS_TERMINATION_GRACE * 5;
                    loop {
                        if child.try_wait()?.is_some() {
                            break;
                        }
                        if Instant::now() >= grace_deadline {
                            let _ = terminate_process_group(child_pid, true);
                            let _ = child.wait()?;
                            break;
                        }
                        thread::sleep(PROCESS_POLL_INTERVAL);
                    }
                } else if let Err(error) = terminate_process_group(child_pid, false) {
                    let _ = child.wait();
                    return Err(error);
                }
                tree_terminated = true;
            } else {
                child.kill()?;
            }
            break Some(child.wait()?);
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    };
    let reaped = status.is_some();
    join_reader(stdout_join)?;
    join_reader(stderr_join)?;
    let mut output = output
        .lock()
        .map_err(|_| io::Error::other("process output lock was poisoned"))?;
    let supervisor = supervisor_status.map(|path| match read_supervisor_status(path) {
        Ok(status) => status,
        Err(error) => SupervisorStatus {
            termination_error: Some(format!("supervisor status unavailable: {error}")),
            ..SupervisorStatus::default()
        },
    });
    Ok(CommandCapture {
        status,
        stdout: std::mem::take(&mut output.stdout),
        stderr: std::mem::take(&mut output.stderr),
        spawn_error: None,
        timed_out,
        tree_terminated,
        reaped,
        supervisor,
    })
}

fn read_supervisor_status(path: &Path) -> io::Result<SupervisorStatus> {
    let text = fs::read_to_string(path)?;
    let mut status = SupervisorStatus::default();
    let mut saw_tree_terminated = false;
    let mut saw_reaped = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed supervisor status",
            ));
        };
        match key {
            "tree_terminated" => {
                status.tree_terminated = parse_bool(value)?;
                saw_tree_terminated = true;
            }
            "reaped" => {
                status.reaped = parse_bool(value)?;
                saw_reaped = true;
            }
            "descendants_detected" => status.descendants_detected = parse_bool(value)?,
            "survivor_count" => {
                status.survivor_count = value.parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid survivor count")
                })?;
            }
            "capability_failure" => status.capability_failure = Some(value.to_owned()),
            "termination_error" => status.termination_error = Some(value.to_owned()),
            "target_code" => {
                status.target_code = Some(value.parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid target code")
                })?);
            }
            "target_signal" => {
                status.target_signal = Some(value.parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid target signal")
                })?);
            }
            "spawn_error" => status.spawn_error = Some(value.to_owned()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown supervisor status field",
                ));
            }
        }
    }
    if !saw_tree_terminated || !saw_reaped {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete supervisor status",
        ));
    }
    Ok(status)
}

fn parse_bool(value: &str) -> io::Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid supervisor boolean",
        )),
    }
}

fn spawn_reader<R>(
    mut reader: R,
    output: Arc<Mutex<SharedOutput>>,
    stdout: bool,
) -> JoinHandle<io::Result<()>>
where
    R: io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let mut output = output
                .lock()
                .map_err(|_| io::Error::other("process output lock was poisoned"))?;
            let retained = output.stdout.bytes.len() + output.stderr.bytes.len();
            let remaining = output.limit.saturating_sub(retained);
            let target = if stdout {
                &mut output.stdout
            } else {
                &mut output.stderr
            };
            let keep = remaining.min(count);
            target.bytes.extend_from_slice(&buffer[..keep]);
            if keep < count {
                target.truncated = true;
            }
        }
        Ok(())
    })
}

fn join_reader(join: JoinHandle<io::Result<()>>) -> io::Result<()> {
    join.join()
        .map_err(|_| io::Error::other("process output reader panicked"))?
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_group_exists(pid: u32) -> bool {
    Pid::from_raw(pid as i32).is_some_and(|pid| test_kill_process_group(pid).is_ok())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_group_exists(_pid: u32) -> bool {
    false
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn terminate_process_group(pid: u32, force: bool) -> io::Result<()> {
    let Some(pid) = Pid::from_raw(pid as i32) else {
        return Ok(());
    };
    let signal = if force { Signal::KILL } else { Signal::TERM };
    if let Err(error) = kill_process_group(pid, signal) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error.into());
        }
    }
    if !force {
        thread::sleep(PROCESS_TERMINATION_GRACE);
        if process_group_exists(pid.as_raw_pid() as u32) {
            terminate_process_group(pid.as_raw_pid() as u32, true)?;
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn terminate_process_group(_pid: u32, _force: bool) -> io::Result<()> {
    Ok(())
}

fn write_process_artifacts(
    fixture: &mut LifecycleFixture,
    case: &RunnerCase,
    process: &ProcessCapture,
) -> Result<(), RunnerError> {
    let store = ArtifactStore::new(fixture.roots().artifacts())?;
    let prefix = Path::new(case.id.as_str());
    store.write_bytes(prefix.join("stdout.bin"), &process.stdout.bytes)?;
    store.write_bytes(prefix.join("stderr.bin"), &process.stderr.bytes)?;
    let status = format!(
        "code={:?}\nsignal={:?}\nspawn_error={:?}\ntimed_out={}\ntree_terminated={}\nreaped={}\ndescendants_detected={}\ntermination_error={:?}\nstdout_truncated={}\nstderr_truncated={}\nstdout_redacted={}\nstderr_redacted={}\nstdout_control_escaped={}\nstderr_control_escaped={}\nstdout_non_utf8={}\nstderr_non_utf8={}\n",
        process.code,
        process.signal,
        process.spawn_error,
        process.timed_out,
        process.tree_terminated,
        process.reaped,
        process.descendants_detected,
        process.termination_error,
        process.stdout.truncated,
        process.stderr.truncated,
        process.stdout.redacted,
        process.stderr.redacted,
        process.stdout.control_escaped,
        process.stderr.control_escaped,
        process.stdout.non_utf8,
        process.stderr.non_utf8,
    );
    store.write_bytes(prefix.join("status.txt"), status.as_bytes())?;
    fixture.record(
        "e2e.process.capture",
        format!(
            "case={};stdout_bytes={};stderr_bytes={};stdout_truncated={};stderr_truncated={};timed_out={};tree_terminated={};reaped={}",
            case.id.as_str(),
            process.stdout.bytes.len(),
            process.stderr.bytes.len(),
            process.stdout.truncated,
            process.stderr.truncated,
            process.timed_out,
            process.tree_terminated,
            process.reaped
        ),
    );
    Ok(())
}

#[cfg(unix)]
fn signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn signal(_status: &ExitStatus) -> Option<i32> {
    None
}

fn metadata_for_path(
    fixture: &LifecycleFixture,
    path: &Path,
) -> Result<ArtifactMetadata, RunnerError> {
    let path = fixture.roots().confine(path)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(RunnerError::Build {
            binary: path.display().to_string(),
            reason: "fixture executable is not a regular file".to_owned(),
        });
    }
    Ok(ArtifactMetadata {
        identity: RelativePathIdentity::from_path(fixture.roots().root(), &path)?,
        relative_path: relative_path(fixture.roots().root(), &path),
        size: metadata.len(),
        fingerprint: fingerprint_file(&path)?,
    })
}

fn collect_artifacts(
    fixture: &LifecycleFixture,
    root: &Path,
) -> Result<Vec<ArtifactMetadata>, RunnerError> {
    let mut paths = Vec::new();
    collect_regular_files(fixture, root, &mut paths)?;
    let mut artifacts = Vec::with_capacity(paths.len());
    for path in paths {
        artifacts.push(metadata_for_path(fixture, &path)?);
    }
    artifacts.sort();
    Ok(artifacts)
}

fn collect_regular_files(
    _fixture: &LifecycleFixture,
    root: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), RunnerError> {
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !root_metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            collect_regular_files(_fixture, &path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn collect_files(
    fixture: &LifecycleFixture,
    root: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), RunnerError> {
    fixture.roots().confine(root)?;
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        fixture.roots().confine(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            collect_files(fixture, &path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        } else {
            return Err(RunnerError::InvalidCase(format!(
                "fixture produced a non-regular artifact: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn check_expectations(
    fixture: &LifecycleFixture,
    case: &RunnerCase,
    process: &ProcessCapture,
    effects: &Path,
) -> Result<(), String> {
    let expected = &case.expected;
    if let Some(error) = &process.spawn_error {
        return Err(format!("spawn failed: {error}"));
    }
    if let Some(error) = &process.termination_error {
        return Err(format!("strict process supervision failed: {error}"));
    }
    if !process.tree_terminated || !process.reaped {
        return Err(format!(
            "strict process supervision incomplete: tree_terminated={} reaped={}",
            process.tree_terminated, process.reaped
        ));
    }
    if process.timed_out {
        return Err("process timed out".to_owned());
    }
    if let Some(code) = expected.exit_code {
        if process.code != Some(code) || process.signal.is_some() {
            return Err(format!(
                "exit status expected code={code}, observed code={:?} signal={:?}",
                process.code, process.signal
            ));
        }
    }
    if let Some(stdout) = &expected.stdout {
        if process.stdout.bytes != *stdout {
            return Err(format!(
                "stdout mismatch: expected {} bytes, observed {} bytes",
                stdout.len(),
                process.stdout.bytes.len()
            ));
        }
    }
    if let Some(stderr) = &expected.stderr {
        if process.stderr.bytes != *stderr {
            return Err(format!(
                "stderr mismatch: expected {} bytes, observed {} bytes",
                stderr.len(),
                process.stderr.bytes.len()
            ));
        }
    }

    let observed = effect_files(fixture, effects).map_err(|error| error.to_string())?;
    let observed_set = observed
        .iter()
        .map(|path| path.identity.clone())
        .collect::<BTreeSet<_>>();
    if let Some(exact) = &expected.exact_files {
        let expected_set = exact
            .iter()
            .map(|file| file.identity.clone())
            .collect::<BTreeSet<_>>();
        if observed_set != expected_set {
            return Err(format!(
                "effect set mismatch: expected {:?}, observed {:?}",
                display_identities(&expected_set),
                display_identities(&observed_set)
            ));
        }
        for file in exact {
            check_expected_file(fixture, effects, file)?;
        }
    }
    for file in &expected.required_files {
        if !observed_set.contains(&file.identity) {
            return Err(format!(
                "required effect is missing: {:?}",
                file.relative_path
            ));
        }
        check_expected_file(fixture, effects, file)?;
    }
    for forbidden in &expected.forbidden_files {
        if observed_set.contains(&RelativePathIdentity::from_text(forbidden)) {
            return Err(format!("forbidden effect exists: {forbidden:?}"));
        }
    }
    Ok(())
}

fn display_identities(identities: &BTreeSet<RelativePathIdentity>) -> Vec<String> {
    identities
        .iter()
        .map(RelativePathIdentity::display)
        .collect()
}

fn git_ref_changes_unexpected(git: &GitFixtureMetadata, expected: &ExpectedGitChanges) -> bool {
    [
        (GitRoot::Source, &git.source_before, &git.source_after),
        (
            GitRoot::Destination,
            &git.destination_before,
            &git.destination_after,
        ),
        (GitRoot::Remote, &git.remote_before, &git.remote_after),
    ]
    .into_iter()
    .any(|(root, before, after)| {
        let actual = git_ref_delta(before, after);
        actual != *expected.for_root(root)
    })
}

fn git_ref_delta(before: &GitRefState, after: &GitRefState) -> BTreeMap<String, Option<String>> {
    let mut names = BTreeSet::new();
    names.extend(before.refs.keys().cloned());
    names.extend(after.refs.keys().cloned());
    if before.head != after.head {
        names.insert("HEAD".to_owned());
    }
    if before.symbolic_head != after.symbolic_head {
        names.insert("HEAD@symbolic".to_owned());
    }
    names
        .into_iter()
        .filter_map(|name| {
            if name == "HEAD" {
                return Some((name, after.head.clone()));
            }
            if name == "HEAD@symbolic" {
                return Some((name, after.symbolic_head.clone()));
            }
            let before_value = before.refs.get(&name);
            let after_value = after.refs.get(&name);
            (before_value != after_value).then(|| (name, after_value.cloned()))
        })
        .collect()
}

fn effect_files(
    fixture: &LifecycleFixture,
    root: &Path,
) -> Result<Vec<ArtifactMetadata>, RunnerError> {
    if !root.is_dir() {
        return Err(RunnerError::InvalidCase(format!(
            "effect root is not a directory: {}",
            root.display()
        )));
    }
    let mut paths = Vec::new();
    collect_files(fixture, root, &mut paths)?;
    let root = fixture.roots().confine(root)?;
    let mut files = Vec::new();
    for path in paths {
        let metadata = metadata_for_path(fixture, &path)?;
        let relative = path
            .strip_prefix(&root)
            .map_err(|_| RunnerError::InvalidCase("effect path escaped root".to_owned()))?;
        let identity = relative_identity(relative)?;
        files.push(ArtifactMetadata {
            relative_path: identity.display(),
            identity,
            ..metadata
        });
    }
    files.sort();
    Ok(files)
}

fn check_expected_file(
    fixture: &LifecycleFixture,
    effects: &Path,
    expected: &ExpectedFile,
) -> Result<(), String> {
    let path = expected
        .identity
        .to_path(effects)
        .map_err(|error| error.to_string())?;
    fixture
        .roots()
        .confine(&path)
        .map_err(|error| error.to_string())?;
    if let Some(contents) = &expected.contents {
        if !file_matches(path.as_path(), contents).map_err(|error| error.to_string())? {
            return Err(format!(
                "effect bytes mismatch for {:?}: expected {} bytes, observed {} bytes",
                expected.relative_path,
                contents.len(),
                fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .len()
            ));
        }
    }
    Ok(())
}

fn file_matches(path: &Path, expected: &[u8]) -> io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let mut buffer = [0_u8; 8192];
    let mut offset = 0_usize;
    loop {
        let count = io::Read::read(&mut file, &mut buffer)?;
        if count == 0 {
            return Ok(offset == expected.len());
        }
        let end = offset.saturating_add(count);
        if end > expected.len() || buffer[..count] != expected[offset..end] {
            return Ok(false);
        }
        offset = end;
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|relative| relative_identity(relative).ok())
        .map(|identity| identity.display())
        .unwrap_or_else(|| "<outside>".to_owned())
}
