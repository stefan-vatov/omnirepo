//! Repository-owned feature-test orchestration.
//!
//! This module owns selection, scheduling, process isolation, and terminal
//! accounting.  It does not define product journeys, replay policy, or the
//! normative quality gates.  Journey definitions are supplied by a versioned
//! manifest (the `.74.7` target is one consumer), evidence is delegated to
//! `omnirepo-test-support`, and quality is delegated to [`crate::quality`].

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt, fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use omnirepo_test_support::failure_replay::ReplayCommand;
use omnirepo_test_support::test_evidence::{
    ArtifactReference, ArtifactStore, DiagnosticRedactor, EventRecorder, EvidenceError,
    MAX_EVIDENCE_BYTES, Outcome, SourcePlanConfig, TestIdentity, sanitize_channels,
};
use serde::{Deserialize, Serialize};

/// Versioned input accepted by the private repository test command.
pub const TEST_SUITE_MANIFEST_SCHEMA: &str = "omnirepo.test-suite-manifest.v1";
/// Versioned aggregate report emitted by the private repository test command.
pub const TEST_SUITE_REPORT_SCHEMA: &str = "omnirepo.test-suite-report.v1";
/// Event records use the evidence module's shared event schema.
pub const TEST_SUITE_EVENT_SCHEMA: &str = "omnirepo.test-event.v1";
/// Versioned replay reference retained by the orchestrator for `.74.5`.
pub const TEST_SUITE_REPLAY_SCHEMA: &str = "omnirepo.test-replay-reference.v1";
/// Default bounded capture for each worker output channel.
pub const MAX_WORKER_OUTPUT_BYTES: usize = MAX_EVIDENCE_BYTES;
const DEFAULT_JOBS: usize = 1;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const OUTPUT_TRUNCATION_MARKER: &[u8] = b"\n[worker output truncated]\n";
const PROCESS_TIMEOUT_EXIT: i32 = 124;
const UNSUPPORTED_EXIT: i32 = 125;
const MISSING_TOOL_EXIT: i32 = 127;
const HARNESS_EXIT: i32 = 1;

/// Cooperative cancellation shared by the command owner and its workers.
///
/// The command-line owner can connect this token to its signal or lifecycle
/// boundary. Cancelling closes useful work admission, terminates active
/// process trees, and gives every queued case a terminal cancellation result.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// The supported feature-test views.  These values are taxonomy labels, not
/// product policy and do not create a second traceability matrix.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteKind {
    Unit,
    Component,
    E2e,
    Adversarial,
    Platform,
}

impl SuiteKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Component => "component",
            Self::E2e => "e2e",
            Self::Adversarial => "adversarial",
            Self::Platform => "platform",
        }
    }
}

impl fmt::Display for SuiteKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Explicit selection of one case, one suite, or the complete manifest.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Selection {
    Case(String),
    Suite(String),
    #[default]
    Full,
}

impl Selection {
    pub fn parse(
        case_id: Option<&str>,
        suite_id: Option<&str>,
        full: bool,
    ) -> Result<Self, RunnerError> {
        let selected =
            usize::from(case_id.is_some()) + usize::from(suite_id.is_some()) + usize::from(full);
        if selected > 1 {
            return Err(RunnerError::InvalidSelection {
                reason: "--case, --suite, and --full are mutually exclusive".to_owned(),
            });
        }
        if let Some(case_id) = case_id {
            return Ok(Self::Case(case_id.to_owned()));
        }
        if let Some(suite_id) = suite_id {
            return Ok(Self::Suite(suite_id.to_owned()));
        }
        Ok(Self::Full)
    }

    pub fn label(&self) -> String {
        match self {
            Self::Case(case_id) => format!("case:{case_id}"),
            Self::Suite(suite_id) => format!("suite:{suite_id}"),
            Self::Full => "full".to_owned(),
        }
    }
}

/// One capability result declared by a case. Unsupported capabilities are
/// terminalized as typed non-green outcomes; they are never silently omitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySpec {
    pub name: String,
    pub supported: bool,
    #[serde(default)]
    pub detail: Option<String>,
}

/// One explicitly declared process invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseSpec {
    pub id: String,
    pub argv: Vec<String>,
    #[serde(default = "default_working_directory")]
    pub working_directory: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<CapabilitySpec>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_working_directory() -> String {
    ".".to_owned()
}

/// A taxonomy view and its ordered cases.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteSpec {
    pub id: String,
    pub kind: SuiteKind,
    pub cases: Vec<CaseSpec>,
}

/// A reference to the existing repository quality authority.  The test
/// command consumes this reference only when `--quality-profile` is selected.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualityReference {
    pub manifest: String,
    #[serde(default)]
    pub default_profile: Option<String>,
}

/// Strict, repository-owned test-suite manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteManifest {
    pub schema: String,
    pub version: u64,
    pub suites: Vec<SuiteSpec>,
    #[serde(default)]
    pub quality: Option<QualityReference>,
}

/// Inputs for one orchestrated run.
#[derive(Clone, Debug)]
pub struct RunnerOptions {
    pub manifest_path: PathBuf,
    pub repo_root: PathBuf,
    pub artifact_root: Option<PathBuf>,
    pub selection: Selection,
    pub jobs: usize,
    pub quality_manifest: Option<PathBuf>,
    pub quality_profile: Option<String>,
    pub cancellation: Option<CancellationToken>,
}

impl RunnerOptions {
    pub fn new(manifest_path: impl Into<PathBuf>, repo_root: impl Into<PathBuf>) -> Self {
        Self {
            manifest_path: manifest_path.into(),
            repo_root: repo_root.into(),
            artifact_root: None,
            selection: Selection::Full,
            jobs: DEFAULT_JOBS,
            quality_manifest: None,
            quality_profile: None,
            cancellation: None,
        }
    }

    pub fn with_artifacts(mut self, artifact_root: impl Into<PathBuf>) -> Self {
        self.artifact_root = Some(artifact_root.into());
        self
    }

    pub fn with_selection(mut self, selection: Selection) -> Self {
        self.selection = selection;
        self
    }

    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.jobs = jobs;
        self
    }

    pub fn with_quality_manifest(mut self, manifest: impl Into<PathBuf>) -> Self {
        self.quality_manifest = Some(manifest.into());
        self
    }

    pub fn with_quality_profile(mut self, profile: impl Into<String>) -> Self {
        self.quality_profile = Some(profile.into());
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
}

/// Terminal status for one selected case.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseOutcome {
    Passed,
    Failed,
    UnsupportedCapability,
    /// The case requires a host-bound capability (for example a filesystem
    /// family) that this host does not provide.  The case is a recorded,
    /// visible skip, not a failure: the manifest is shared across hosts and
    /// the host resolves the reality.
    HostUnsupported,
    MissingTool,
    TimedOut,
    HarnessFailure,
    Cancelled,
}

impl CaseOutcome {
    pub const fn success(self) -> bool {
        matches!(self, Self::Passed | Self::HostUnsupported)
    }

    fn evidence_outcome(self) -> Outcome {
        match self {
            Self::Passed => Outcome::Passed,
            Self::UnsupportedCapability | Self::HostUnsupported => Outcome::Skipped,
            Self::Failed | Self::TimedOut => Outcome::Failed,
            Self::MissingTool | Self::HarnessFailure | Self::Cancelled => Outcome::HarnessFailure,
        }
    }
}

/// Host-bound capability names and the hosts that provide them.  A capability
/// name that is not in this table is resolved solely by the manifest's
/// declaration.
fn host_support(name: &str) -> Option<bool> {
    match name {
        "linux-ext-family" => Some(cfg!(target_os = "linux")),
        "macos-apfs" => Some(cfg!(target_os = "macos")),
        _ => None,
    }
}

/// A stable argv replay reference.  `.74.5` owns full replay bundle policy;
/// this record only preserves the orchestrator's invocation identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayReference {
    pub schema: String,
    pub replay_id: String,
    pub argv: Vec<String>,
    pub recipe: String,
    pub artifact: String,
    pub replayable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One selected case result. Worker channels are artifact paths, never
/// terminal output fields, so worker chatter cannot contaminate the CLI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub id: String,
    pub suite: String,
    pub kind: SuiteKind,
    pub outcome: CaseOutcome,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub artifact: String,
    pub replay: Option<ReplayReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

/// Summary of delegated `.63/.34` quality execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualityResult {
    pub delegated: bool,
    pub profile: String,
    pub success: bool,
    pub exit_code: i32,
    pub failed_gates: Vec<String>,
    pub artifact: String,
}

/// Stable aggregate result for local and CI invocations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestSuiteReport {
    pub schema: String,
    pub selection: Selection,
    pub suites: Vec<String>,
    pub success: bool,
    pub exit_code: i32,
    pub event_log: String,
    pub artifact_root: String,
    pub cases: Vec<CaseResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<QualityResult>,
    pub report: String,
}

/// Typed failures that prevent an aggregate report from being produced.
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
    InvalidSelection {
        reason: String,
    },
    UnknownCase {
        case_id: String,
    },
    UnknownSuite {
        suite_id: String,
    },
    InvalidOptions {
        reason: String,
    },
    InvalidRepositoryRoot {
        path: PathBuf,
        source: io::Error,
    },
    InvalidArtifactRoot {
        path: PathBuf,
        source: EvidenceError,
    },
    InvalidWorkingDirectory {
        case_id: String,
        path: PathBuf,
        reason: String,
    },
    Evidence(EvidenceError),
    Artifact {
        path: String,
        source: EvidenceError,
    },
    Serialize {
        context: &'static str,
        source: serde_json::Error,
    },
    Quality {
        message: String,
    },
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadManifest { path, source } => {
                write!(
                    formatter,
                    "cannot read test-suite manifest {}: {source}",
                    path.display()
                )
            }
            Self::ParseManifest { path, source } => {
                write!(
                    formatter,
                    "cannot parse test-suite manifest {}: {source}",
                    path.display()
                )
            }
            Self::InvalidManifest { path, reason } => {
                write!(
                    formatter,
                    "invalid test-suite manifest {}: {reason}",
                    path.display()
                )
            }
            Self::InvalidSelection { reason } => {
                write!(formatter, "invalid test selection: {reason}")
            }
            Self::UnknownCase { case_id } => write!(formatter, "unknown test case {case_id:?}"),
            Self::UnknownSuite { suite_id } => write!(formatter, "unknown test suite {suite_id:?}"),
            Self::InvalidOptions { reason } => {
                write!(formatter, "invalid test-suite options: {reason}")
            }
            Self::InvalidRepositoryRoot { path, source } => {
                write!(
                    formatter,
                    "cannot resolve repository root {}: {source}",
                    path.display()
                )
            }
            Self::InvalidArtifactRoot { path, source } => {
                write!(
                    formatter,
                    "cannot prepare artifact root {}: {source}",
                    path.display()
                )
            }
            Self::InvalidWorkingDirectory {
                case_id,
                path,
                reason,
            } => {
                write!(
                    formatter,
                    "invalid working directory for case {case_id:?} ({}): {reason}",
                    path.display()
                )
            }
            Self::Evidence(error) => write!(formatter, "test evidence error: {error}"),
            Self::Artifact { path, source } => {
                write!(formatter, "cannot write test artifact {path:?}: {source}")
            }
            Self::Serialize { context, source } => {
                write!(formatter, "cannot serialize {context}: {source}")
            }
            Self::Quality { message } => {
                write!(formatter, "delegated quality run failed: {message}")
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
            Self::ParseManifest { source, .. } | Self::Serialize { source, .. } => Some(source),
            Self::Evidence(source) | Self::InvalidArtifactRoot { source, .. } => Some(source),
            Self::Artifact { source, .. } => Some(source),
            Self::InvalidManifest { .. }
            | Self::InvalidSelection { .. }
            | Self::UnknownCase { .. }
            | Self::UnknownSuite { .. }
            | Self::InvalidOptions { .. }
            | Self::InvalidWorkingDirectory { .. }
            | Self::Quality { .. } => None,
        }
    }
}

impl From<EvidenceError> for RunnerError {
    fn from(error: EvidenceError) -> Self {
        Self::Evidence(error)
    }
}

#[derive(Clone)]
struct PreparedCase {
    index: usize,
    suite: String,
    kind: SuiteKind,
    case: CaseSpec,
    identity: TestIdentity,
    artifact: ArtifactReference,
    artifact_path: String,
    replay: ReplayReference,
    working_directory: PathBuf,
    case_root: PathBuf,
}

#[derive(Clone, Debug)]
struct ProcessResult {
    outcome: CaseOutcome,
    exit_code: Option<i32>,
    signal: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    diagnostic: Option<String>,
    duration_ms: u64,
}

/// Run the selected cases and optionally delegate one existing quality
/// profile.  Every selected case is terminalized, including failures after a
/// peer has failed.
pub fn run(options: &RunnerOptions) -> Result<TestSuiteReport, RunnerError> {
    if options.jobs == 0 {
        return Err(RunnerError::InvalidOptions {
            reason: "jobs must be greater than zero".to_owned(),
        });
    }

    let source =
        fs::read_to_string(&options.manifest_path).map_err(|source| RunnerError::ReadManifest {
            path: options.manifest_path.clone(),
            source,
        })?;
    let manifest = serde_json::from_str::<SuiteManifest>(&source).map_err(|source| {
        RunnerError::ParseManifest {
            path: options.manifest_path.clone(),
            source,
        }
    })?;
    validate_manifest(&options.manifest_path, &manifest)?;

    let repo_root =
        options
            .repo_root
            .canonicalize()
            .map_err(|source| RunnerError::InvalidRepositoryRoot {
                path: options.repo_root.clone(),
                source,
            })?;
    let artifact_root = options
        .artifact_root
        .clone()
        .unwrap_or_else(|| repo_root.join("target/omnirepo-test-artifacts"));
    let store =
        ArtifactStore::new(&artifact_root).map_err(|source| RunnerError::InvalidArtifactRoot {
            path: artifact_root.clone(),
            source,
        })?;
    let run_id = unique_run_id();
    let run_root = store
        .resolve(&run_id)
        .map_err(|source| RunnerError::InvalidArtifactRoot {
            path: artifact_root.clone(),
            source,
        })?;
    fs::create_dir_all(&run_root).map_err(|source| RunnerError::InvalidArtifactRoot {
        path: run_root.clone(),
        source: EvidenceError::Io(source),
    })?;

    let selected = select_cases(&manifest, &options.selection)?;
    let suite_ids = selected
        .iter()
        .map(|case| case.suite.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let redactor = DiagnosticRedactor::new(
        manifest
            .suites
            .iter()
            .flat_map(|suite| suite.cases.iter())
            .flat_map(|case| case.environment.values().cloned()),
    );
    let recorder = EventRecorder::new(redactor.clone());
    let source_plan_config = SourcePlanConfig::new(
        "repository-test-suite-manifest",
        options.selection.label(),
        "omnirepo.test-suite.v1",
    )?;

    let mut prepared = Vec::with_capacity(selected.len());
    for (index, selected_case) in selected.into_iter().enumerate() {
        let working_directory = resolve_working_directory(&repo_root, &selected_case.case)?;
        let artifact_path = format!(
            "{run_id}/cases/{}/{}/result.json",
            selected_case.suite, selected_case.case.id
        );
        let replay_path = format!(
            "{run_id}/cases/{}/{}/replay.json",
            selected_case.suite, selected_case.case.id
        );
        let replay_id = format!("replay-{}-{}", selected_case.suite, selected_case.case.id);
        let replay_command = ReplayCommand::new(
            selected_case.case.argv[0].clone(),
            selected_case
                .case
                .argv
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<_>>(),
        );
        let replay = ReplayReference {
            schema: TEST_SUITE_REPLAY_SCHEMA.to_owned(),
            replay_id: replay_id.clone(),
            argv: selected_case.case.argv.clone(),
            recipe: replay_command.render(),
            artifact: replay_path,
            replayable: true,
            reason: None,
        };
        let case_artifact = ArtifactReference::new(&artifact_path, replay_id)?;
        let identity = TestIdentity::new(
            selected_case.case.id.clone(),
            selected_case.suite.clone(),
            "omnirepo",
            selected_case.suite.clone(),
            source_plan_config.clone(),
            1,
            selected_case.case.seed,
            selected_case.kind.as_str(),
        )?;
        recorder.expect(identity.clone())?;
        let case_root = store
            .resolve(format!(
                "{run_id}/cases/{}/{}",
                selected_case.suite, selected_case.case.id
            ))
            .map_err(|source| RunnerError::Artifact {
                path: artifact_path.clone(),
                source,
            })?;
        prepared.push(PreparedCase {
            index,
            suite: selected_case.suite,
            kind: selected_case.kind,
            case: selected_case.case,
            identity,
            artifact: case_artifact,
            artifact_path,
            replay,
            working_directory,
            case_root,
        });
    }

    let jobs = options.jobs.min(prepared.len().max(1));
    let queue = Arc::new(Mutex::new(VecDeque::from(prepared.clone())));
    let results = Arc::new(Mutex::new(vec![None; prepared.len()]));
    let cancelled = options.cancellation.clone().unwrap_or_default().cancelled;
    let cancelled = Arc::clone(&cancelled);
    let mut workers = Vec::with_capacity(jobs);
    for _ in 0..jobs {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);
        let recorder = recorder.clone();
        let store = store.clone();
        let redactor = redactor.clone();
        let cancelled = Arc::clone(&cancelled);
        workers.push(thread::spawn(move || {
            loop {
                let next = queue
                    .lock()
                    .expect("test queue mutex is not poisoned")
                    .pop_front();
                let Some(case) = next else { break };
                let result = execute_case(&case, &recorder, &store, &redactor, &cancelled);
                results.lock().expect("test results mutex is not poisoned")[case.index] =
                    Some(result);
            }
        }));
    }
    for worker in workers {
        if worker.join().is_err() {
            cancelled.store(true, Ordering::Release);
        }
    }

    let evidence = recorder.finalize()?;
    let event_log = format!("{run_id}/events.jsonl");
    store
        .write_bytes(&event_log, evidence.to_jsonl()?.as_bytes())
        .map_err(|source| RunnerError::Artifact {
            path: event_log.clone(),
            source,
        })?;

    let cases = results
        .lock()
        .expect("test results mutex is not poisoned")
        .iter()
        .enumerate()
        .map(|(index, result)| {
            result
                .clone()
                .unwrap_or_else(|| missing_worker_result(&prepared[index]))
        })
        .collect::<Vec<_>>();
    // Preserve manifest order.  It is the explicit selection order and the
    // deterministic tie-break for the suite's first failure status.

    let delegated_quality = delegate_quality(options, &manifest, &repo_root, &store, &run_id)?;
    let case_exit = cases
        .iter()
        .find(|case| !case.success)
        .map(failure_exit_code);
    let quality_exit = delegated_quality
        .as_ref()
        .filter(|quality| !quality.success)
        .map(|quality| quality.exit_code);
    let exit_code = case_exit.or(quality_exit).unwrap_or(0);
    let success = cases.iter().all(|case| case.success)
        && delegated_quality
            .as_ref()
            .is_none_or(|quality| quality.success);
    let report_path = format!("{run_id}/report.json");
    let report = TestSuiteReport {
        schema: TEST_SUITE_REPORT_SCHEMA.to_owned(),
        selection: options.selection.clone(),
        suites: suite_ids,
        success,
        exit_code,
        event_log,
        artifact_root: run_id,
        cases,
        quality: delegated_quality,
        report: report_path.clone(),
    };
    let report_json = serde_json::to_vec(&report).map_err(|source| RunnerError::Serialize {
        context: "test-suite report",
        source,
    })?;
    store
        .write_bytes(&report_path, &report_json)
        .map_err(|source| RunnerError::Artifact {
            path: report_path,
            source,
        })?;
    Ok(report)
}

fn unique_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("run-{nanos:x}-{}", std::process::id())
}

fn failure_exit_code(case: &CaseResult) -> i32 {
    case.exit_code
        .or_else(|| case.signal.and_then(|signal| signal.checked_add(128)))
        .unwrap_or(HARNESS_EXIT)
}

fn validate_manifest(path: &Path, manifest: &SuiteManifest) -> Result<(), RunnerError> {
    if manifest.schema != TEST_SUITE_MANIFEST_SCHEMA || manifest.version != 1 {
        return Err(RunnerError::InvalidManifest {
            path: path.to_owned(),
            reason: format!("schema must be {TEST_SUITE_MANIFEST_SCHEMA} version 1"),
        });
    }
    if manifest.suites.is_empty() {
        return Err(RunnerError::InvalidManifest {
            path: path.to_owned(),
            reason: "suites must not be empty".to_owned(),
        });
    }
    let mut suite_ids = BTreeSet::new();
    let mut case_ids = BTreeSet::new();
    for suite in &manifest.suites {
        validate_slug("suite id", &suite.id).map_err(|reason| RunnerError::InvalidManifest {
            path: path.to_owned(),
            reason,
        })?;
        if !suite_ids.insert(suite.id.clone()) {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("suite ID {:?} is duplicated", suite.id),
            });
        }
        if suite.cases.is_empty() {
            return Err(RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("suite {:?} has no cases", suite.id),
            });
        }
        for case in &suite.cases {
            validate_slug("case id", &case.id).map_err(|reason| RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason,
            })?;
            if !case_ids.insert(case.id.clone()) {
                return Err(RunnerError::InvalidManifest {
                    path: path.to_owned(),
                    reason: format!("case ID {:?} is duplicated", case.id),
                });
            }
            if case.argv.is_empty() || case.argv[0].is_empty() {
                return Err(RunnerError::InvalidManifest {
                    path: path.to_owned(),
                    reason: format!("case {:?} must declare a non-empty argv", case.id),
                });
            }
            validate_relative_path(&case.working_directory).map_err(|reason| {
                RunnerError::InvalidManifest {
                    path: path.to_owned(),
                    reason: format!("case {:?}: {reason}", case.id),
                }
            })?;
            if case.timeout_seconds == Some(0) {
                return Err(RunnerError::InvalidManifest {
                    path: path.to_owned(),
                    reason: format!(
                        "case {:?} timeout_seconds must be greater than zero",
                        case.id
                    ),
                });
            }
            let mut capability_names = BTreeSet::new();
            for capability in &case.capabilities {
                if capability.name.is_empty() || capability.name.chars().any(char::is_control) {
                    return Err(RunnerError::InvalidManifest {
                        path: path.to_owned(),
                        reason: format!("case {:?} has an invalid capability name", case.id),
                    });
                }
                if !capability_names.insert(capability.name.clone()) {
                    return Err(RunnerError::InvalidManifest {
                        path: path.to_owned(),
                        reason: format!(
                            "case {:?} repeats capability {:?}",
                            case.id, capability.name
                        ),
                    });
                }
                if capability.supported && capability.detail.is_some() {
                    return Err(RunnerError::InvalidManifest {
                        path: path.to_owned(),
                        reason: format!(
                            "case {:?} supported capability {:?} cannot have detail",
                            case.id, capability.name
                        ),
                    });
                }
                if !capability.supported && capability.detail.as_deref().is_none_or(str::is_empty) {
                    return Err(RunnerError::InvalidManifest {
                        path: path.to_owned(),
                        reason: format!(
                            "case {:?} unsupported capability {:?} needs detail",
                            case.id, capability.name
                        ),
                    });
                }
            }
            for key in case.environment.keys() {
                if key.is_empty()
                    || key.chars().any(|character| {
                        character.is_control() || character == '=' || character == '\0'
                    })
                {
                    return Err(RunnerError::InvalidManifest {
                        path: path.to_owned(),
                        reason: format!("case {:?} has an invalid environment key", case.id),
                    });
                }
                if matches!(
                    key.as_str(),
                    "HOME" | "TMPDIR" | "TMP" | "TEMP" | "OMNIREPO_TEST_ROOT"
                ) {
                    return Err(RunnerError::InvalidManifest {
                        path: path.to_owned(),
                        reason: format!(
                            "case {:?} cannot override isolated environment key {key:?}",
                            case.id
                        ),
                    });
                }
            }
        }
    }
    if let Some(quality) = &manifest.quality {
        validate_relative_path(&quality.manifest).map_err(|reason| {
            RunnerError::InvalidManifest {
                path: path.to_owned(),
                reason: format!("quality manifest: {reason}"),
            }
        })?;
    }
    Ok(())
}

fn validate_slug(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(format!(
            "{label} must be a non-empty lowercase path-safe slug: {value:?}"
        ));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute() || value.is_empty() {
        return Err(format!(
            "path must be a non-empty repository-relative path: {value:?}"
        ));
    }
    for component in path.components() {
        match component {
            Component::Normal(value) if !value.is_empty() => {}
            Component::Normal(_) => {
                return Err(format!("path contains an empty component: {value:?}"));
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path must not escape its root: {value:?}"));
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
struct SelectedCase {
    suite: String,
    kind: SuiteKind,
    case: CaseSpec,
}

fn select_cases(
    manifest: &SuiteManifest,
    selection: &Selection,
) -> Result<Vec<SelectedCase>, RunnerError> {
    let mut selected = Vec::new();
    for suite in &manifest.suites {
        for case in &suite.cases {
            let include = match selection {
                Selection::Full => true,
                Selection::Case(case_id) => &case.id == case_id,
                Selection::Suite(suite_id) => &suite.id == suite_id,
            };
            if include {
                selected.push(SelectedCase {
                    suite: suite.id.clone(),
                    kind: suite.kind,
                    case: case.clone(),
                });
            }
        }
    }
    if selected.is_empty() {
        return match selection {
            Selection::Case(case_id) => Err(RunnerError::UnknownCase {
                case_id: case_id.clone(),
            }),
            Selection::Suite(suite_id) => Err(RunnerError::UnknownSuite {
                suite_id: suite_id.clone(),
            }),
            Selection::Full => Err(RunnerError::InvalidManifest {
                path: PathBuf::new(),
                reason: "full selection contains no cases".to_owned(),
            }),
        };
    }
    Ok(selected)
}

fn resolve_working_directory(repo_root: &Path, case: &CaseSpec) -> Result<PathBuf, RunnerError> {
    let candidate = repo_root.join(&case.working_directory);
    let resolved =
        candidate
            .canonicalize()
            .map_err(|source| RunnerError::InvalidWorkingDirectory {
                case_id: case.id.clone(),
                path: candidate.clone(),
                reason: source.to_string(),
            })?;
    if !resolved.starts_with(repo_root) {
        return Err(RunnerError::InvalidWorkingDirectory {
            case_id: case.id.clone(),
            path: candidate,
            reason: "working directory escapes repo root".to_owned(),
        });
    }
    Ok(resolved)
}

fn execute_case(
    case: &PreparedCase,
    recorder: &EventRecorder,
    store: &ArtifactStore,
    redactor: &DiagnosticRedactor,
    cancelled: &AtomicBool,
) -> CaseResult {
    let started = Instant::now();
    let guard = recorder.start(case.identity.clone(), case.artifact.clone());
    if guard.is_err() {
        return harness_result(case, "could not start evidence step");
    }
    let mut result = if cancelled.load(Ordering::Acquire) {
        ProcessResult {
            outcome: CaseOutcome::Cancelled,
            exit_code: Some(130),
            signal: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            diagnostic: Some("case was cancelled before dispatch".to_owned()),
            duration_ms: 0,
        }
    } else if let Some(capability) = case.capabilities().find(|capability| !capability.supported) {
        ProcessResult {
            outcome: CaseOutcome::UnsupportedCapability,
            exit_code: Some(UNSUPPORTED_EXIT),
            signal: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            diagnostic: Some(format!(
                "unsupported capability {}: {}",
                capability.name,
                capability.detail.as_deref().unwrap_or("not available")
            )),
            duration_ms: 0,
        }
    } else if let Some(capability) = case
        .capabilities()
        .find(|capability| host_support(&capability.name) == Some(false))
    {
        ProcessResult {
            outcome: CaseOutcome::HostUnsupported,
            exit_code: None,
            signal: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            diagnostic: Some(format!(
                "host does not support capability {} (host {}): case skipped",
                capability.name,
                std::env::consts::OS
            )),
            duration_ms: 0,
        }
    } else {
        execute_process(case, cancelled)
    };

    let stdout_path = format!(
        "{}/stdout.log",
        case.artifact_path.trim_end_matches("/result.json")
    );
    let stderr_path = format!(
        "{}/stderr.log",
        case.artifact_path.trim_end_matches("/result.json")
    );
    let channels = sanitize_channels(redactor, &result.stdout, &result.stderr, MAX_EVIDENCE_BYTES);
    if let Ok(channels) = channels {
        if store
            .write_bytes(&stdout_path, channels.stdout.text.as_bytes())
            .is_err()
            || store
                .write_bytes(&stderr_path, channels.stderr.text.as_bytes())
                .is_err()
        {
            result.outcome = CaseOutcome::HarnessFailure;
            result.exit_code = Some(HARNESS_EXIT);
            result.diagnostic = Some("worker output artifact could not be written".to_owned());
        }
    } else {
        result.outcome = CaseOutcome::HarnessFailure;
        result.exit_code = Some(HARNESS_EXIT);
        result.diagnostic = Some("worker output could not be sanitized".to_owned());
    }

    if let Ok(bytes) = serde_json::to_vec(&case.replay) {
        if store.write_bytes(&case.replay.artifact, &bytes).is_err() {
            result.outcome = CaseOutcome::HarnessFailure;
            result.exit_code = Some(HARNESS_EXIT);
            result.diagnostic = Some("replay reference artifact could not be written".to_owned());
        }
    } else {
        result.outcome = CaseOutcome::HarnessFailure;
        result.exit_code = Some(HARNESS_EXIT);
        result.diagnostic = Some("replay reference could not be serialized".to_owned());
    }
    let replay = Some(case.replay.clone());
    let case_result = CaseResult {
        id: case.case.id.clone(),
        suite: case.suite.clone(),
        kind: case.kind,
        outcome: result.outcome,
        success: result.outcome.success(),
        exit_code: result.exit_code,
        signal: result.signal,
        duration_ms: result.duration_ms.max(started.elapsed().as_millis() as u64),
        stdout: stdout_path.clone(),
        stderr: stderr_path.clone(),
        artifact: case.artifact_path.clone(),
        replay,
        diagnostic: result
            .diagnostic
            .as_deref()
            .map(|diagnostic| redactor.sanitize(diagnostic).text),
    };
    let result_path = &case.artifact_path;
    if let Ok(bytes) = serde_json::to_vec(&case_result) {
        if store.write_bytes(result_path, &bytes).is_err() {
            result = ProcessResult {
                outcome: CaseOutcome::HarnessFailure,
                exit_code: Some(HARNESS_EXIT),
                signal: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                diagnostic: Some("case result artifact could not be written".to_owned()),
                duration_ms: case_result.duration_ms,
            };
            let _ = guard
                .expect("evidence guard exists")
                .harness_failure("case result artifact could not be written");
            return CaseResult {
                id: case.case.id.clone(),
                suite: case.suite.clone(),
                kind: case.kind,
                outcome: result.outcome,
                success: false,
                exit_code: result.exit_code,
                signal: None,
                duration_ms: result.duration_ms,
                stdout: stdout_path,
                stderr: stderr_path,
                artifact: case.artifact_path.clone(),
                replay: Some(case.replay.clone()),
                diagnostic: result.diagnostic,
            };
        }
    }

    let guard = guard.expect("evidence guard exists");
    let evidence_outcome = result.outcome.evidence_outcome();
    let evidence_diagnostic = result.diagnostic.clone();
    match evidence_outcome {
        Outcome::Passed => {
            let _ = guard.pass();
        }
        Outcome::Skipped => {
            let _ = guard.skip(
                evidence_diagnostic
                    .as_deref()
                    .unwrap_or("unsupported capability"),
            );
        }
        Outcome::Failed => {
            let _ = guard.fail(evidence_diagnostic.as_deref().unwrap_or("test case failed"));
        }
        Outcome::HarnessFailure => {
            let _ = guard.harness_failure(
                evidence_diagnostic
                    .as_deref()
                    .unwrap_or("test harness failure"),
            );
        }
        Outcome::Started => {
            let _ = guard.harness_failure("invalid non-terminal worker outcome");
        }
    }
    case_result
}

impl PreparedCase {
    fn capabilities(&self) -> impl Iterator<Item = &CapabilitySpec> {
        self.case.capabilities.iter()
    }
}

fn harness_result(case: &PreparedCase, diagnostic: &str) -> CaseResult {
    CaseResult {
        id: case.case.id.clone(),
        suite: case.suite.clone(),
        kind: case.kind,
        outcome: CaseOutcome::HarnessFailure,
        success: false,
        exit_code: Some(HARNESS_EXIT),
        signal: None,
        duration_ms: 0,
        stdout: format!(
            "{}/stdout.log",
            case.artifact_path.trim_end_matches("/result.json")
        ),
        stderr: format!(
            "{}/stderr.log",
            case.artifact_path.trim_end_matches("/result.json")
        ),
        artifact: case.artifact_path.clone(),
        replay: Some(case.replay.clone()),
        diagnostic: Some(diagnostic.to_owned()),
    }
}

fn missing_worker_result(case: &PreparedCase) -> CaseResult {
    harness_result(case, "worker did not return a terminal result")
}

fn execute_process(case: &PreparedCase, cancelled: &AtomicBool) -> ProcessResult {
    let started = Instant::now();
    let mut command = Command::new(&case.case.argv[0]);
    command
        .args(case.case.argv.iter().skip(1))
        .current_dir(&case.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    let home = case.case_root.join("home");
    let temp = case.case_root.join("tmp");
    let artifacts = case.case_root.join("artifacts");
    let _ = fs::create_dir_all(&home);
    let _ = fs::create_dir_all(&temp);
    let _ = fs::create_dir_all(&artifacts);
    command
        .env("HOME", &home)
        .env("TMPDIR", &temp)
        .env("TMP", &temp)
        .env("TEMP", &temp)
        .env("OMNIREPO_TEST_ROOT", &case.case_root)
        .env("OMNIREPO_TEST_HOME", &home)
        .env("OMNIREPO_TEST_ARTIFACTS", &artifacts);
    for key in ["PATH", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    for (key, value) in &case.case.environment {
        command.env(key, value);
    }
    configure_process_group(&mut command);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ProcessResult {
                outcome: CaseOutcome::MissingTool,
                exit_code: Some(MISSING_TOOL_EXIT),
                signal: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                diagnostic: Some(format!("tool is unavailable: {}", case.case.argv[0])),
                duration_ms: elapsed_ms(started),
            };
        }
        Err(error) => {
            return ProcessResult {
                outcome: CaseOutcome::HarnessFailure,
                exit_code: Some(HARNESS_EXIT),
                signal: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                diagnostic: Some(format!("process could not start: {error}")),
                duration_ms: elapsed_ms(started),
            };
        }
    };
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_thread = thread::spawn(|| read_capped(stdout));
    let stderr_thread = thread::spawn(|| read_capped(stderr));
    let timeout = case
        .case
        .timeout_seconds
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_TIMEOUT);
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let mut cancelled_process = false;
    let status = loop {
        if cancelled.load(Ordering::Acquire) {
            cancelled_process = true;
            terminate_process_tree(&mut child);
            let status = child
                .wait()
                .unwrap_or_else(|_| process_error_status(&io::Error::other("cancellation")));
            break status;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                terminate_process_tree(&mut child);
                let status = child
                    .wait()
                    .unwrap_or_else(|_| process_error_status(&error));
                break status;
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            terminate_process_tree(&mut child);
            let status = child
                .wait()
                .unwrap_or_else(|error| process_error_status(&error));
            break status;
        }
        thread::park_timeout(Duration::from_millis(5));
    };
    let stdout = stdout_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stdout reader panicked")))
        .unwrap_or_else(|error| format!("capture error: {error}").into_bytes());
    let stderr = stderr_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stderr reader panicked")))
        .unwrap_or_else(|error| format!("capture error: {error}").into_bytes());
    let signal = status_signal(status);
    let exit_code = status.code();
    let outcome = if cancelled_process {
        CaseOutcome::Cancelled
    } else if timed_out {
        CaseOutcome::TimedOut
    } else if status.success() {
        CaseOutcome::Passed
    } else {
        CaseOutcome::Failed
    };
    let diagnostic = if cancelled_process {
        Some("process was cancelled and its process group was terminated".to_owned())
    } else if timed_out {
        Some("process timed out and its process group was terminated".to_owned())
    } else if outcome == CaseOutcome::Failed {
        Some(match exit_code {
            Some(code) => format!("process exited with status {code}"),
            None => "process terminated by signal".to_owned(),
        })
    } else {
        None
    };
    ProcessResult {
        outcome,
        exit_code: if cancelled_process {
            Some(130)
        } else if timed_out {
            Some(PROCESS_TIMEOUT_EXIT)
        } else {
            exit_code
        },
        signal,
        stdout,
        stderr,
        diagnostic,
        duration_ms: elapsed_ms(started),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn read_capped<R: Read>(mut reader: R) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = MAX_WORKER_OUTPUT_BYTES.saturating_sub(output.len());
        let take = remaining.min(count);
        output.extend_from_slice(&buffer[..take]);
        truncated |= take != count;
    }
    if truncated {
        let marker_budget = OUTPUT_TRUNCATION_MARKER.len();
        if output.len() + marker_budget > MAX_WORKER_OUTPUT_BYTES {
            output.truncate(MAX_WORKER_OUTPUT_BYTES.saturating_sub(marker_budget));
        }
        output.extend_from_slice(OUTPUT_TRUNCATION_MARKER);
    }
    Ok(output)
}

fn delegate_quality(
    options: &RunnerOptions,
    manifest: &SuiteManifest,
    repo_root: &Path,
    store: &ArtifactStore,
    run_id: &str,
) -> Result<Option<QualityResult>, RunnerError> {
    let Some(profile) = options.quality_profile.clone().or_else(|| {
        manifest
            .quality
            .as_ref()
            .and_then(|quality| quality.default_profile.clone())
    }) else {
        return Ok(None);
    };
    let manifest_path = options
        .quality_manifest
        .clone()
        .or_else(|| {
            manifest
                .quality
                .as_ref()
                .map(|quality| PathBuf::from(&quality.manifest))
        })
        .unwrap_or_else(|| PathBuf::from("scripts/quality-manifest.json"));
    let manifest_path = resolve_repo_relative(repo_root, &manifest_path).map_err(|message| {
        RunnerError::Quality {
            message: format!("quality manifest path is invalid: {message}"),
        }
    })?;
    let quality_options =
        crate::quality::RunnerOptions::new(manifest_path, repo_root).with_profile(profile.clone());
    let quality_report =
        crate::quality::run(&quality_options).map_err(|error| RunnerError::Quality {
            message: error.to_string(),
        })?;
    let failed_gates = quality_report
        .gates
        .iter()
        .filter(|gate| !gate.success)
        .map(|gate| gate.id.clone())
        .collect::<Vec<_>>();
    let artifact = format!("{run_id}/quality.json");
    let json =
        crate::quality::render_json(&quality_report).map_err(|error| RunnerError::Quality {
            message: error.to_string(),
        })?;
    store
        .write_bytes(&artifact, json.as_bytes())
        .map_err(|source| RunnerError::Artifact {
            path: artifact.clone(),
            source,
        })?;
    Ok(Some(QualityResult {
        delegated: true,
        profile,
        success: quality_report.success,
        exit_code: quality_report.exit_code,
        failed_gates,
        artifact,
    }))
}

fn resolve_repo_relative(repo_root: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err("absolute paths are not supported".to_owned());
    }
    let candidate = repo_root.join(path);
    let resolved = candidate
        .canonicalize()
        .map_err(|error| format!("{}: {error}", candidate.display()))?;
    if !resolved.starts_with(repo_root) {
        return Err(format!("{} escapes repository root", path.display()));
    }
    Ok(resolved)
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = child.id() as i32;
        unsafe {
            let _ = killpg(process_group, 9);
        }
    }
    let _ = child.kill();
}

fn process_error_status(error: &io::Error) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(error.raw_os_error().unwrap_or(HARNESS_EXIT) << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(error.raw_os_error().unwrap_or(HARNESS_EXIT) as u32)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        Command::new("true")
            .status()
            .expect("platform has a process status implementation")
    }
}

fn status_signal(status: ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn killpg(pgrp: i32, sig: i32) -> i32;
}
