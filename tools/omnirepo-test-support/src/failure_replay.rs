//! Deterministic failure replay and diagnostic bundles.
//!
//! This module is a persistence boundary for test failures. It contains no
//! process, network, Git, clock, or HOME access. A runner supplies explicit
//! fixture facts and receives either a bounded, sanitized bundle or a typed
//! error. Replaying is a pure comparison against a new observation; executing
//! a command remains the responsibility of the hermetic runner.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::path::{Component, Path};

use serde::{Deserialize, Deserializer, Serialize, de::Error as DeError};

use crate::test_evidence::{
    ArtifactReference, ArtifactStore, DiagnosticRedactor, EventKind, EventRecorder, EvidenceBundle,
    EvidenceError, MAX_EVIDENCE_BYTES, SourcePlanConfig, TestIdentity, sanitize_channels,
};

pub use crate::test_evidence::Outcome;

/// Version of the persisted failure-replay bundle.
pub const FAILURE_REPLAY_SCHEMA: &str = "omnirepo.failure-replay.v1";
/// Maximum serialized size of one replay bundle, including metadata.
pub const MAX_FAILURE_REPLAY_BYTES: usize = MAX_EVIDENCE_BYTES;
const MAX_FIELD_BYTES: usize = 4096;
const MAX_ITEMS: usize = 4096;
const REDACTED_MARKER: &str = "[REDACTED]";

/// Typed errors from bundle construction, parsing, verification, and storage.
#[derive(Debug)]
pub enum FailureReplayError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    InvalidPath(String),
    FailureClassMismatch {
        expected: FailureClass,
        observed: Outcome,
    },
    MissingReplayability,
    BundleTooLarge {
        bytes: usize,
        max: usize,
    },
    Evidence(EvidenceError),
    Json(serde_json::Error),
    BundleWriteFailed {
        path: String,
        source: EvidenceError,
    },
}

impl Display for FailureReplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => {
                write!(
                    formatter,
                    "invalid failure-replay field {field:?}: {reason}"
                )
            }
            Self::InvalidPath(path) => write!(formatter, "invalid replay path: {path}"),
            Self::FailureClassMismatch { expected, observed } => write!(
                formatter,
                "failure class {expected} does not match observed outcome {observed:?}"
            ),
            Self::MissingReplayability => {
                formatter.write_str("failed case has no replayability disposition")
            }
            Self::BundleTooLarge { bytes, max } => write!(
                formatter,
                "failure-replay bundle is {bytes} bytes, above the {max}-byte bound"
            ),
            Self::Evidence(error) => Display::fmt(error, formatter),
            Self::Json(error) => write!(formatter, "failure-replay JSON error: {error}"),
            Self::BundleWriteFailed { path, source } => {
                write!(
                    formatter,
                    "failed to persist replay bundle {path:?}: {source}"
                )
            }
        }
    }
}

impl std::error::Error for FailureReplayError {}

impl From<EvidenceError> for FailureReplayError {
    fn from(error: EvidenceError) -> Self {
        Self::Evidence(error)
    }
}

impl From<serde_json::Error> for FailureReplayError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// The product or harness context that produced a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    HarnessFailure,
    ProductFailure,
    UnsupportedCapability,
}

impl Display for FailureClass {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HarnessFailure => "harness_failure",
            Self::ProductFailure => "product_failure",
            Self::UnsupportedCapability => "unsupported_capability",
        })
    }
}

/// Failure points for which the existing hermetic controls provide replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureScenario {
    ProcessCrash,
    ConcurrentRun,
    InterruptedJournal,
    AmbiguousGitDelivery,
    RepairAttempt,
    PartialSourceAvailability,
}

/// Why a failed case cannot be replayed from a saved bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonReplayableReason {
    MissingSeed,
    MissingBarrierSchedule,
    MissingEventLog,
    NondeterministicInput,
    UnsupportedPlatform,
    ExternalServiceRequired,
    AmbientStateRequired,
    CorruptEvidence,
    BundleCreationFailed,
}

/// A replay disposition. A replayable case always contains one command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Replayability {
    Replayable { command: ReplayCommand },
    NonReplayable { reason: NonReplayableReason },
}

/// A deterministic command represented as argv, never as an unescaped shell
/// fragment. render creates the one-command recipe for a human or agent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayCommand {
    program: String,
    args: Vec<String>,
    recipe: String,
}

impl ReplayCommand {
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let program = program.into();
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let recipe = render_command(&program, &args);
        Self {
            program,
            args,
            recipe,
        }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn recipe(&self) -> &str {
        &self.recipe
    }

    /// Render every argv item as one safely quoted POSIX shell word.
    pub fn render(&self) -> String {
        render_command(&self.program, &self.args)
    }

    fn validate(&self) -> Result<(), FailureReplayError> {
        checked_text("replay_command.program", &self.program)?;
        if self.args.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "replay_command.args",
                reason: "too many command arguments",
            });
        }
        for argument in &self.args {
            checked_text("replay_command.arg", argument)?;
        }
        checked_text("replay_command.recipe", &self.recipe)?;
        if self.recipe != self.render() {
            return Err(FailureReplayError::InvalidField {
                field: "replay_command.recipe",
                reason: "recipe must match the deterministic argv rendering",
            });
        }
        Ok(())
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        let program = sanitize_text(redactor, "replay_command.program", &self.program)?;
        let args = self
            .args
            .iter()
            .map(|argument| sanitize_text(redactor, "replay_command.arg", argument))
            .collect::<Result<Vec<_>, _>>()?;
        let command = Self::new(program, args);
        command.validate()?;
        Ok(command)
    }
}

/// Explicit command and configuration facts. No ambient environment is
/// collected by this type; callers provide only the selected summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSummary {
    program: String,
    args: Vec<String>,
    config: BTreeMap<String, String>,
}

impl CommandSummary {
    pub fn new<I, S>(program: impl Into<String>, args: I, config: BTreeMap<String, String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            config,
        }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn config(&self) -> &BTreeMap<String, String> {
        &self.config
    }

    fn validate(&self) -> Result<(), FailureReplayError> {
        checked_text("command.program", &self.program)?;
        if self.args.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "command.args",
                reason: "too many command arguments",
            });
        }
        for argument in &self.args {
            checked_text("command.arg", argument)?;
        }
        if self.config.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "command.config",
                reason: "too many configuration entries",
            });
        }
        for (key, value) in &self.config {
            checked_text("command.config.key", key)?;
            checked_text_allow_empty("command.config.value", value)?;
        }
        Ok(())
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        self.validate()?;
        let program = sanitize_text(redactor, "command.program", &self.program)?;
        let args = self
            .args
            .iter()
            .map(|argument| sanitize_text(redactor, "command.arg", argument))
            .collect::<Result<Vec<_>, _>>()?;
        let mut config = BTreeMap::new();
        for (key, value) in &self.config {
            let key = sanitize_text(redactor, "command.config.key", key)?;
            let value = sanitize_config_value(redactor, &key, value)?;
            if config.insert(key, value).is_some() {
                return Err(FailureReplayError::InvalidField {
                    field: "command.config",
                    reason: "sanitized configuration keys must be unique",
                });
            }
        }
        let summary = Self {
            program,
            args,
            config,
        };
        summary.validate()?;
        Ok(summary)
    }
}

/// A selected capability and its explicit availability result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySnapshot {
    name: String,
    supported: bool,
    detail: Option<String>,
}

impl CapabilitySnapshot {
    pub fn available(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supported: true,
            detail: None,
        }
    }

    pub fn unsupported(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supported: false,
            detail: Some(detail.into()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    fn validate(&self) -> Result<(), FailureReplayError> {
        checked_text("capability.name", &self.name)?;
        if let Some(detail) = &self.detail {
            checked_text_allow_empty("capability.detail", detail)?;
        }
        if self.supported && self.detail.is_some() {
            return Err(FailureReplayError::InvalidField {
                field: "capability.detail",
                reason: "supported capabilities cannot carry an unsupported detail",
            });
        }
        if !self.supported && self.detail.as_deref().is_none_or(str::is_empty) {
            return Err(FailureReplayError::InvalidField {
                field: "capability.detail",
                reason: "unsupported capabilities require a reason",
            });
        }
        Ok(())
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        let snapshot = Self {
            name: sanitize_text(redactor, "capability.name", &self.name)?,
            supported: self.supported,
            detail: self
                .detail
                .as_deref()
                .map(|detail| sanitize_text_allow_empty(redactor, "capability.detail", detail))
                .transpose()?,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

/// Platform and filesystem contract selected for a replay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformContract {
    platform: String,
    capabilities: Vec<CapabilitySnapshot>,
}

impl PlatformContract {
    pub fn new(platform: impl Into<String>, mut capabilities: Vec<CapabilitySnapshot>) -> Self {
        capabilities.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            platform: platform.into(),
            capabilities,
        }
    }

    pub fn platform(&self) -> &str {
        &self.platform
    }

    pub fn capabilities(&self) -> &[CapabilitySnapshot] {
        &self.capabilities
    }

    fn validate(&self) -> Result<(), FailureReplayError> {
        checked_text("platform", &self.platform)?;
        if self.capabilities.is_empty() {
            return Err(FailureReplayError::InvalidField {
                field: "platform.capabilities",
                reason: "at least one selected capability is required",
            });
        }
        if self.capabilities.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "platform.capabilities",
                reason: "too many capability entries",
            });
        }
        let mut previous = None;
        for capability in &self.capabilities {
            capability.validate()?;
            if previous.is_some_and(|name: &str| name >= capability.name.as_str()) {
                return Err(FailureReplayError::InvalidField {
                    field: "platform.capabilities",
                    reason: "capabilities must be sorted and unique",
                });
            }
            previous = Some(capability.name.as_str());
        }
        Ok(())
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        let mut capabilities = self
            .capabilities
            .iter()
            .map(|capability| capability.sanitize(redactor))
            .collect::<Result<Vec<_>, _>>()?;
        capabilities.sort_by(|left, right| left.name.cmp(&right.name));
        let contract = Self {
            platform: sanitize_text(redactor, "platform", &self.platform)?,
            capabilities,
        };
        contract.validate()?;
        Ok(contract)
    }
}

/// One named deterministic barrier transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarrierAction {
    Armed,
    Hit,
    Released,
    Aborted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarrierStep {
    sequence: u64,
    name: String,
    action: BarrierAction,
}

impl BarrierStep {
    pub fn new(sequence: u64, name: impl Into<String>, action: BarrierAction) -> Self {
        Self {
            sequence,
            name: name.into(),
            action,
        }
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn action(&self) -> BarrierAction {
        self.action
    }

    fn validate_schedule(schedule: &[Self]) -> Result<(), FailureReplayError> {
        if schedule.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "barrier_schedule",
                reason: "too many barrier transitions",
            });
        }
        let mut previous = None;
        for step in schedule {
            if step.sequence == 0 {
                return Err(FailureReplayError::InvalidField {
                    field: "barrier_schedule.sequence",
                    reason: "sequence numbers start at one",
                });
            }
            checked_text("barrier_schedule.name", &step.name)?;
            if previous.is_some_and(|value| value >= step.sequence) {
                return Err(FailureReplayError::InvalidField {
                    field: "barrier_schedule",
                    reason: "barrier sequence numbers must be strictly increasing",
                });
            }
            previous = Some(step.sequence);
        }
        Ok(())
    }
}

/// A durable event that happened before the first failing assertion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableEvent {
    sequence: u64,
    kind: String,
    detail: String,
}

impl DurableEvent {
    pub fn new(sequence: u64, kind: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            sequence,
            kind: kind.into(),
            detail: detail.into(),
        }
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    fn sanitize_all(
        events: &[Self],
        first_failure: u64,
        redactor: &DiagnosticRedactor,
    ) -> Result<Vec<Self>, FailureReplayError> {
        if events.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "durable_events",
                reason: "too many durable events",
            });
        }
        let mut events = events
            .iter()
            .map(|event| {
                Ok(Self {
                    sequence: event.sequence,
                    kind: sanitize_text(redactor, "durable_event.kind", &event.kind)?,
                    detail: sanitize_text_allow_empty(
                        redactor,
                        "durable_event.detail",
                        &event.detail,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, FailureReplayError>>()?;
        events.sort_by_key(|event| event.sequence);
        let mut previous = None;
        for event in &events {
            if event.sequence == 0 || event.sequence >= first_failure {
                return Err(FailureReplayError::InvalidField {
                    field: "durable_events.sequence",
                    reason: "durable events must precede the first failure",
                });
            }
            if previous.is_some_and(|value| value >= event.sequence) {
                return Err(FailureReplayError::InvalidField {
                    field: "durable_events",
                    reason: "durable event sequence numbers must be unique",
                });
            }
            previous = Some(event.sequence);
        }
        Ok(events)
    }
}

/// The first assertion that failed. Later assertion output is not retained as
/// a substitute for this causal marker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionFailure {
    event_sequence: u64,
    assertion_id: String,
    expected: String,
    observed: String,
}

impl AssertionFailure {
    pub fn new(
        event_sequence: u64,
        assertion_id: impl Into<String>,
        expected: impl Into<String>,
        observed: impl Into<String>,
    ) -> Self {
        Self {
            event_sequence,
            assertion_id: assertion_id.into(),
            expected: expected.into(),
            observed: observed.into(),
        }
    }

    pub fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    pub fn assertion_id(&self) -> &str {
        &self.assertion_id
    }

    pub fn expected(&self) -> &str {
        &self.expected
    }

    pub fn observed(&self) -> &str {
        &self.observed
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        if self.event_sequence == 0 {
            return Err(FailureReplayError::InvalidField {
                field: "first_failure.event_sequence",
                reason: "the first failure sequence starts at one",
            });
        }
        let failure = Self {
            event_sequence: self.event_sequence,
            assertion_id: sanitize_text(
                redactor,
                "first_failure.assertion_id",
                &self.assertion_id,
            )?,
            expected: sanitize_text_allow_empty(
                redactor,
                "first_failure.expected",
                &self.expected,
            )?,
            observed: sanitize_text_allow_empty(
                redactor,
                "first_failure.observed",
                &self.observed,
            )?,
        };
        checked_text("first_failure.assertion_id", &failure.assertion_id)?;
        if failure.expected == failure.observed {
            return Err(FailureReplayError::InvalidField {
                field: "first_failure",
                reason: "expected and observed values must differ",
            });
        }
        Ok(failure)
    }
}

/// One expected-versus-observed effect difference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDifference {
    subject: String,
    expected: Option<String>,
    observed: Option<String>,
}

impl EffectDifference {
    pub fn new(
        subject: impl Into<String>,
        expected: Option<impl Into<String>>,
        observed: Option<impl Into<String>>,
    ) -> Self {
        Self {
            subject: subject.into(),
            expected: expected.map(Into::into),
            observed: observed.map(Into::into),
        }
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn expected(&self) -> Option<&str> {
        self.expected.as_deref()
    }

    pub fn observed(&self) -> Option<&str> {
        self.observed.as_deref()
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        let difference = Self {
            subject: safe_relative_text(redactor, "effect_diff.subject", &self.subject)?,
            expected: self
                .expected
                .as_deref()
                .map(|value| sanitize_text_allow_empty(redactor, "effect_diff.expected", value))
                .transpose()?,
            observed: self
                .observed
                .as_deref()
                .map(|value| sanitize_text_allow_empty(redactor, "effect_diff.observed", value))
                .transpose()?,
        };
        if difference.expected == difference.observed {
            return Err(FailureReplayError::InvalidField {
                field: "effect_diff.entry",
                reason: "expected and observed effects must differ",
            });
        }
        Ok(difference)
    }
}

/// Ordered effect differences. Equal snapshots are represented by no entry.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDiff {
    entries: Vec<EffectDifference>,
}

impl EffectDiff {
    pub fn new(mut entries: Vec<EffectDifference>) -> Self {
        entries.sort_by(|left, right| left.subject.cmp(&right.subject));
        Self { entries }
    }

    pub fn entries(&self) -> &[EffectDifference] {
        &self.entries
    }

    fn sanitize(&self, redactor: &DiagnosticRedactor) -> Result<Self, FailureReplayError> {
        if self.entries.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "effect_diff.entries",
                reason: "too many effect differences",
            });
        }
        let mut entries = self
            .entries
            .iter()
            .map(|entry| entry.sanitize(redactor))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by(|left, right| left.subject.cmp(&right.subject));
        let mut previous = None;
        for entry in &entries {
            if previous.is_some_and(|subject: &str| subject >= entry.subject.as_str()) {
                return Err(FailureReplayError::InvalidField {
                    field: "effect_diff.entries",
                    reason: "effect subjects must be unique and sorted",
                });
            }
            previous = Some(entry.subject.as_str());
        }
        Ok(Self { entries })
    }
}

/// One peer case's terminal outcome, retained even when another case fails.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerOutcome {
    case_id: String,
    outcome: Outcome,
    diagnostic: Option<String>,
}

impl PeerOutcome {
    pub fn new(case_id: impl Into<String>, outcome: Outcome, diagnostic: Option<String>) -> Self {
        Self {
            case_id: case_id.into(),
            outcome,
            diagnostic,
        }
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    fn sanitize_all(
        peers: &[Self],
        redactor: &DiagnosticRedactor,
    ) -> Result<Vec<Self>, FailureReplayError> {
        if peers.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "peer_outcomes",
                reason: "too many peer outcomes",
            });
        }
        let mut peers = peers
            .iter()
            .map(|peer| {
                if peer.outcome == Outcome::Started {
                    return Err(FailureReplayError::InvalidField {
                        field: "peer_outcomes.outcome",
                        reason: "peer outcomes must be terminal",
                    });
                }
                Ok(Self {
                    case_id: sanitize_text(redactor, "peer_outcomes.case_id", &peer.case_id)?,
                    outcome: peer.outcome,
                    diagnostic: peer
                        .diagnostic
                        .as_deref()
                        .map(|diagnostic| {
                            sanitize_text_allow_empty(
                                redactor,
                                "peer_outcomes.diagnostic",
                                diagnostic,
                            )
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, FailureReplayError>>()?;
        peers.sort_by(|left, right| left.case_id.cmp(&right.case_id));
        let mut previous = None;
        for peer in &peers {
            checked_text("peer_outcomes.case_id", &peer.case_id)?;
            if previous.is_some_and(|case_id: &str| case_id >= peer.case_id.as_str()) {
                return Err(FailureReplayError::InvalidField {
                    field: "peer_outcomes",
                    reason: "peer case IDs must be unique and sorted",
                });
            }
            previous = Some(peer.case_id.as_str());
        }
        Ok(peers)
    }
}

/// Cleanup errors are separate from the first product assertion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupFailure {
    stage: String,
    diagnostic: String,
}

impl CleanupFailure {
    pub fn new(stage: impl Into<String>, diagnostic: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            diagnostic: diagnostic.into(),
        }
    }

    pub fn stage(&self) -> &str {
        &self.stage
    }

    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    fn sanitize_all(
        failures: &[Self],
        redactor: &DiagnosticRedactor,
    ) -> Result<Vec<Self>, FailureReplayError> {
        if failures.len() > MAX_ITEMS {
            return Err(FailureReplayError::InvalidField {
                field: "cleanup_failures",
                reason: "too many cleanup failures",
            });
        }
        let mut failures = failures
            .iter()
            .map(|failure| {
                Ok(Self {
                    stage: sanitize_text(redactor, "cleanup_failure.stage", &failure.stage)?,
                    diagnostic: sanitize_text_allow_empty(
                        redactor,
                        "cleanup_failure.diagnostic",
                        &failure.diagnostic,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, FailureReplayError>>()?;
        failures.sort_by(|left, right| {
            left.stage
                .cmp(&right.stage)
                .then_with(|| left.diagnostic.cmp(&right.diagnostic))
        });
        Ok(failures)
    }
}

/// The complete input assembled by a test runner before persistence.
#[derive(Clone, Debug)]
pub struct FailureReplaySpec {
    manifest_version: String,
    case_id: String,
    fixture_id: String,
    scenario: FailureScenario,
    failure_class: FailureClass,
    seed: u64,
    evidence: EvidenceBundle,
    platform: Option<PlatformContract>,
    command: Option<CommandSummary>,
    event_log_path: Option<String>,
    barriers: Vec<BarrierStep>,
    first_failure: Option<AssertionFailure>,
    durable_events: Vec<DurableEvent>,
    effect_diff: Option<EffectDiff>,
    peers: Vec<PeerOutcome>,
    cleanup_failures: Vec<CleanupFailure>,
    replayability: Option<Replayability>,
}

impl FailureReplaySpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest_version: impl Into<String>,
        case_id: impl Into<String>,
        fixture_id: impl Into<String>,
        scenario: FailureScenario,
        failure_class: FailureClass,
        seed: u64,
        evidence: EvidenceBundle,
    ) -> Self {
        Self {
            manifest_version: manifest_version.into(),
            case_id: case_id.into(),
            fixture_id: fixture_id.into(),
            scenario,
            failure_class,
            seed,
            evidence,
            platform: None,
            command: None,
            event_log_path: None,
            barriers: Vec::new(),
            first_failure: None,
            durable_events: Vec::new(),
            effect_diff: None,
            peers: Vec::new(),
            cleanup_failures: Vec::new(),
            replayability: None,
        }
    }

    pub fn platform(mut self, platform: PlatformContract) -> Self {
        self.platform = Some(platform);
        self
    }

    pub fn command(mut self, command: CommandSummary) -> Self {
        self.command = Some(command);
        self
    }

    pub fn event_log(mut self, path: impl Into<String>) -> Self {
        self.event_log_path = Some(path.into());
        self
    }

    pub fn barriers(mut self, barriers: Vec<BarrierStep>) -> Self {
        self.barriers = barriers;
        self
    }

    pub fn first_failure(mut self, first_failure: AssertionFailure) -> Self {
        self.first_failure = Some(first_failure);
        self
    }

    pub fn durable_events(mut self, durable_events: Vec<DurableEvent>) -> Self {
        self.durable_events = durable_events;
        self
    }

    pub fn effect_diff(mut self, effect_diff: EffectDiff) -> Self {
        self.effect_diff = Some(effect_diff);
        self
    }

    pub fn peers(mut self, peers: Vec<PeerOutcome>) -> Self {
        self.peers = peers;
        self
    }

    pub fn cleanup_failures(mut self, failures: Vec<CleanupFailure>) -> Self {
        self.cleanup_failures = failures;
        self
    }

    pub fn replayable(mut self, command: ReplayCommand) -> Self {
        self.replayability = Some(Replayability::Replayable { command });
        self
    }

    pub fn non_replayable(mut self, reason: NonReplayableReason) -> Self {
        self.replayability = Some(Replayability::NonReplayable { reason });
        self
    }

    pub fn build(
        self,
        redactor: &DiagnosticRedactor,
    ) -> Result<FailureReplayBundle, FailureReplayError> {
        let evidence = sanitize_evidence_bundle(self.evidence, redactor)?;
        let observed = observed_outcome(&evidence);
        if !class_matches(self.failure_class, &evidence) {
            return Err(FailureReplayError::FailureClassMismatch {
                expected: self.failure_class,
                observed,
            });
        }
        let platform = self
            .platform
            .ok_or(FailureReplayError::InvalidField {
                field: "platform",
                reason: "selected platform capabilities are required",
            })?
            .sanitize(redactor)?;
        let command = self
            .command
            .ok_or(FailureReplayError::InvalidField {
                field: "command",
                reason: "sanitized command/config summary is required",
            })?
            .sanitize(redactor)?;
        let event_log_path = self
            .event_log_path
            .ok_or(FailureReplayError::InvalidField {
                field: "event_log_path",
                reason: "event-log path is required",
            })?;
        let event_log_path = sanitize_text(redactor, "event_log_path", &event_log_path)?;
        let event_log_path = checked_relative_path(&event_log_path)?;
        let mut barriers = self.barriers;
        for barrier in &mut barriers {
            barrier.name = sanitize_text(redactor, "barrier_schedule.name", &barrier.name)?;
        }
        barriers.sort_by_key(|step| step.sequence);
        BarrierStep::validate_schedule(&barriers)?;
        let first_failure = self
            .first_failure
            .ok_or(FailureReplayError::InvalidField {
                field: "first_failure",
                reason: "first failing assertion is required",
            })?
            .sanitize(redactor)?;
        let durable_events = DurableEvent::sanitize_all(
            &self.durable_events,
            first_failure.event_sequence,
            redactor,
        )?;
        let effect_diff = self
            .effect_diff
            .ok_or(FailureReplayError::InvalidField {
                field: "effect_diff",
                reason: "expected-versus-observed effect diff is required",
            })?
            .sanitize(redactor)?;
        let peers = PeerOutcome::sanitize_all(&self.peers, redactor)?;
        let cleanup_failures = CleanupFailure::sanitize_all(&self.cleanup_failures, redactor)?;
        let replayability = self
            .replayability
            .ok_or(FailureReplayError::MissingReplayability)?;
        let replayability = sanitize_replayability(replayability, redactor)?;
        let bundle = FailureReplayBundle {
            schema: FAILURE_REPLAY_SCHEMA.to_owned(),
            manifest_version: sanitize_text(redactor, "manifest_version", &self.manifest_version)?,
            case_id: sanitize_text(redactor, "case_id", &self.case_id)?,
            fixture_id: sanitize_text(redactor, "fixture_id", &self.fixture_id)?,
            scenario: self.scenario,
            failure_class: self.failure_class,
            seed: self.seed,
            platform,
            command,
            event_log_path,
            barriers,
            first_failure,
            durable_events,
            effect_diff,
            peers,
            cleanup_failures,
            replayability,
            evidence,
        };
        bundle.validate()?;
        Ok(bundle)
    }
}

/// A persisted failure bundle. Every public accessor returns data that has
/// already passed the redaction and bound checks at construction or parsing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FailureReplayBundle {
    schema: String,
    manifest_version: String,
    case_id: String,
    fixture_id: String,
    scenario: FailureScenario,
    failure_class: FailureClass,
    seed: u64,
    platform: PlatformContract,
    command: CommandSummary,
    event_log_path: String,
    barriers: Vec<BarrierStep>,
    first_failure: AssertionFailure,
    durable_events: Vec<DurableEvent>,
    effect_diff: EffectDiff,
    peers: Vec<PeerOutcome>,
    cleanup_failures: Vec<CleanupFailure>,
    replayability: Replayability,
    evidence: EvidenceBundle,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureReplayBundleWire {
    schema: String,
    manifest_version: String,
    case_id: String,
    fixture_id: String,
    scenario: FailureScenario,
    failure_class: FailureClass,
    seed: u64,
    platform: PlatformContract,
    command: CommandSummary,
    event_log_path: String,
    barriers: Vec<BarrierStep>,
    first_failure: AssertionFailure,
    durable_events: Vec<DurableEvent>,
    effect_diff: EffectDiff,
    peers: Vec<PeerOutcome>,
    cleanup_failures: Vec<CleanupFailure>,
    replayability: Replayability,
    evidence: EvidenceBundle,
}

impl<'de> Deserialize<'de> for FailureReplayBundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FailureReplayBundleWire::deserialize(deserializer)?;
        let bundle = Self {
            schema: wire.schema,
            manifest_version: wire.manifest_version,
            case_id: wire.case_id,
            fixture_id: wire.fixture_id,
            scenario: wire.scenario,
            failure_class: wire.failure_class,
            seed: wire.seed,
            platform: wire.platform,
            command: wire.command,
            event_log_path: wire.event_log_path,
            barriers: wire.barriers,
            first_failure: wire.first_failure,
            durable_events: wire.durable_events,
            effect_diff: wire.effect_diff,
            peers: wire.peers,
            cleanup_failures: wire.cleanup_failures,
            replayability: wire.replayability,
            evidence: wire.evidence,
        };
        bundle.validate().map_err(D::Error::custom)?;
        Ok(bundle)
    }
}

impl FailureReplayBundle {
    pub fn manifest_version(&self) -> &str {
        &self.manifest_version
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub fn fixture_id(&self) -> &str {
        &self.fixture_id
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub fn scenario(&self) -> FailureScenario {
        self.scenario
    }

    pub fn failure_class(&self) -> FailureClass {
        self.failure_class
    }

    pub fn platform(&self) -> &PlatformContract {
        &self.platform
    }

    pub fn command_summary(&self) -> &CommandSummary {
        &self.command
    }

    pub fn event_log_path(&self) -> &str {
        &self.event_log_path
    }

    pub fn barriers(&self) -> &[BarrierStep] {
        &self.barriers
    }

    pub fn first_failure(&self) -> &AssertionFailure {
        &self.first_failure
    }

    pub fn durable_events(&self) -> &[DurableEvent] {
        &self.durable_events
    }

    pub fn effect_diff(&self) -> &EffectDiff {
        &self.effect_diff
    }

    pub fn peer_outcomes(&self) -> &[PeerOutcome] {
        &self.peers
    }

    pub fn cleanup_failures(&self) -> &[CleanupFailure] {
        &self.cleanup_failures
    }

    pub fn evidence(&self) -> &EvidenceBundle {
        &self.evidence
    }

    pub fn replayability(&self) -> &Replayability {
        &self.replayability
    }

    pub fn replay_command(&self) -> Option<&ReplayCommand> {
        match &self.replayability {
            Replayability::Replayable { command } => Some(command),
            Replayability::NonReplayable { .. } => None,
        }
    }

    pub fn non_replayable_reason(&self) -> Option<NonReplayableReason> {
        match self.replayability {
            Replayability::Replayable { .. } => None,
            Replayability::NonReplayable { reason } => Some(reason),
        }
    }

    /// Serialize deterministically and reject a bundle that exceeds the
    /// complete bounded persistence budget.
    pub fn to_json(&self) -> Result<String, FailureReplayError> {
        self.validate()?;
        let json = serde_json::to_string(self)?;
        if json.len() > MAX_FAILURE_REPLAY_BYTES {
            return Err(FailureReplayError::BundleTooLarge {
                bytes: json.len(),
                max: MAX_FAILURE_REPLAY_BYTES,
            });
        }
        Ok(json)
    }

    pub fn from_json(json: &str) -> Result<Self, FailureReplayError> {
        if json.len() > MAX_FAILURE_REPLAY_BYTES {
            return Err(FailureReplayError::BundleTooLarge {
                bytes: json.len(),
                max: MAX_FAILURE_REPLAY_BYTES,
            });
        }
        Ok(serde_json::from_str(json)?)
    }

    /// Persist through the no-follow, exclusive ArtifactStore boundary. Any
    /// error is returned to the runner and must remain a suite failure.
    pub fn write(
        &self,
        store: &ArtifactStore,
        relative_path: impl AsRef<Path>,
    ) -> Result<(), FailureReplayError> {
        let relative_path = relative_path.as_ref();
        let json = self.to_json()?;
        store
            .write_bytes(relative_path, json.as_bytes())
            .map(|_| ())
            .map_err(|source| FailureReplayError::BundleWriteFailed {
                path: relative_path.to_string_lossy().into_owned(),
                source,
            })
    }

    /// Compare a runner's new pure observation with the saved identity and
    /// event prefix. No external command is launched here.
    pub fn verify_replay(
        &self,
        observation: &ReplayObservation,
    ) -> Result<ReplayVerification, FailureReplayError> {
        self.validate()?;
        if let Replayability::NonReplayable { reason } = self.replayability {
            return Ok(ReplayVerification::NotReplayable { reason });
        }
        let expected = ReplayObservation::from_bundle(self);
        let comparisons = [
            (
                expected.case_id != observation.case_id,
                ReplayDivergence::CaseIdentity,
            ),
            (
                expected.fixture_id != observation.fixture_id,
                ReplayDivergence::FixtureIdentity,
            ),
            (
                expected.failure_class != observation.failure_class,
                ReplayDivergence::FailureClass,
            ),
            (
                expected.scenario != observation.scenario,
                ReplayDivergence::FailureScenario,
            ),
            (expected.seed != observation.seed, ReplayDivergence::Seed),
            (
                expected.platform != observation.platform,
                ReplayDivergence::PlatformContract,
            ),
            (
                expected.barriers != observation.barriers,
                ReplayDivergence::BarrierSchedule,
            ),
            (
                expected.first_failure != observation.first_failure,
                ReplayDivergence::FirstFailure,
            ),
            (
                expected.durable_events != observation.durable_events,
                ReplayDivergence::DurableEventSequence,
            ),
        ];
        for (different, reason) in comparisons {
            if different {
                return Ok(ReplayVerification::Diverged { reason });
            }
        }
        Ok(ReplayVerification::Reproduced)
    }

    fn validate(&self) -> Result<(), FailureReplayError> {
        if self.schema != FAILURE_REPLAY_SCHEMA {
            return Err(FailureReplayError::InvalidField {
                field: "schema",
                reason: "unsupported failure-replay schema",
            });
        }
        checked_text("manifest_version", &self.manifest_version)?;
        checked_text("case_id", &self.case_id)?;
        checked_text("fixture_id", &self.fixture_id)?;
        self.platform.validate()?;
        self.command.validate()?;
        checked_relative_path(&self.event_log_path)?;
        BarrierStep::validate_schedule(&self.barriers)?;
        let mut barriers = self.barriers.clone();
        barriers.sort_by_key(|step| step.sequence);
        if barriers != self.barriers {
            return Err(FailureReplayError::InvalidField {
                field: "barrier_schedule",
                reason: "barriers must be in deterministic order",
            });
        }
        let redactor = DiagnosticRedactor::default();
        for (field, value) in [
            ("manifest_version", &self.manifest_version),
            ("case_id", &self.case_id),
            ("fixture_id", &self.fixture_id),
        ] {
            let sanitized = sanitize_text(&redactor, field, value)?;
            if sanitized != *value {
                return Err(FailureReplayError::InvalidField {
                    field,
                    reason: "identity is not sanitized or bounded",
                });
            }
        }
        let platform = self.platform.sanitize(&redactor)?;
        if platform != self.platform {
            return Err(FailureReplayError::InvalidField {
                field: "platform",
                reason: "platform capability data is not sanitized or bounded",
            });
        }
        let command = self.command.sanitize(&redactor)?;
        if command != self.command {
            return Err(FailureReplayError::InvalidField {
                field: "command",
                reason: "command/config data is not sanitized or bounded",
            });
        }
        let event_log_path = sanitize_text(&redactor, "event_log_path", &self.event_log_path)?;
        if event_log_path != self.event_log_path {
            return Err(FailureReplayError::InvalidField {
                field: "event_log_path",
                reason: "event-log path is not sanitized or bounded",
            });
        }
        for barrier in &self.barriers {
            let name = sanitize_text(&redactor, "barrier_schedule.name", &barrier.name)?;
            if name != barrier.name {
                return Err(FailureReplayError::InvalidField {
                    field: "barrier_schedule.name",
                    reason: "barrier name is not sanitized or bounded",
                });
            }
        }
        let first_failure = self.first_failure.sanitize(&redactor)?;
        if first_failure != self.first_failure {
            return Err(FailureReplayError::InvalidField {
                field: "first_failure",
                reason: "failure assertion is not sanitized or bounded",
            });
        }
        let durable_events = DurableEvent::sanitize_all(
            &self.durable_events,
            self.first_failure.event_sequence,
            &redactor,
        )?;
        if durable_events != self.durable_events {
            return Err(FailureReplayError::InvalidField {
                field: "durable_events",
                reason: "durable events are not sanitized or ordered",
            });
        }
        let effect_diff = self.effect_diff.sanitize(&redactor)?;
        if effect_diff != self.effect_diff {
            return Err(FailureReplayError::InvalidField {
                field: "effect_diff",
                reason: "effect diff is not sanitized or ordered",
            });
        }
        let peers = PeerOutcome::sanitize_all(&self.peers, &redactor)?;
        if peers != self.peers {
            return Err(FailureReplayError::InvalidField {
                field: "peer_outcomes",
                reason: "peer outcomes are not sanitized or ordered",
            });
        }
        let cleanup = CleanupFailure::sanitize_all(&self.cleanup_failures, &redactor)?;
        if cleanup != self.cleanup_failures {
            return Err(FailureReplayError::InvalidField {
                field: "cleanup_failures",
                reason: "cleanup failures are not sanitized or ordered",
            });
        }
        let replayability = sanitize_replayability(self.replayability.clone(), &redactor)?;
        if replayability != self.replayability {
            return Err(FailureReplayError::InvalidField {
                field: "replayability",
                reason: "replayability command is not sanitized or bounded",
            });
        }
        if !class_matches(self.failure_class, &self.evidence) {
            return Err(FailureReplayError::FailureClassMismatch {
                expected: self.failure_class,
                observed: observed_outcome(&self.evidence),
            });
        }
        self.evidence.validate()?;
        Ok(())
    }
}

/// Pure replay input captured by a runner after executing the replay recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayObservation {
    case_id: String,
    fixture_id: String,
    scenario: FailureScenario,
    failure_class: FailureClass,
    seed: u64,
    platform: PlatformContract,
    barriers: Vec<BarrierStep>,
    first_failure: AssertionFailure,
    durable_events: Vec<DurableEvent>,
}

impl ReplayObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        case_id: impl Into<String>,
        fixture_id: impl Into<String>,
        seed: u64,
        platform: PlatformContract,
        barriers: Vec<BarrierStep>,
        first_failure: AssertionFailure,
        durable_events: Vec<DurableEvent>,
    ) -> Self {
        Self {
            case_id: case_id.into(),
            fixture_id: fixture_id.into(),
            scenario: FailureScenario::ProcessCrash,
            failure_class: FailureClass::ProductFailure,
            seed,
            platform,
            barriers,
            first_failure,
            durable_events,
        }
    }

    pub fn from_bundle(bundle: &FailureReplayBundle) -> Self {
        Self {
            case_id: bundle.case_id.clone(),
            fixture_id: bundle.fixture_id.clone(),
            scenario: bundle.scenario,
            failure_class: bundle.failure_class,
            seed: bundle.seed,
            platform: bundle.platform.clone(),
            barriers: bundle.barriers.clone(),
            first_failure: bundle.first_failure.clone(),
            durable_events: bundle.durable_events.clone(),
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn with_scenario(mut self, scenario: FailureScenario) -> Self {
        self.scenario = scenario;
        self
    }

    pub fn with_failure_class(mut self, failure_class: FailureClass) -> Self {
        self.failure_class = failure_class;
        self
    }

    pub const fn scenario(&self) -> FailureScenario {
        self.scenario
    }

    pub const fn failure_class(&self) -> FailureClass {
        self.failure_class
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
}

/// Why a replay differs from its saved contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ReplayDivergence {
    CaseIdentity,
    FixtureIdentity,
    FailureClass,
    FailureScenario,
    Seed,
    PlatformContract,
    BarrierSchedule,
    FirstFailure,
    DurableEventSequence,
}

/// Result of comparing a replay observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayVerification {
    Reproduced,
    Diverged { reason: ReplayDivergence },
    NotReplayable { reason: NonReplayableReason },
}

fn sanitize_evidence_bundle(
    evidence: EvidenceBundle,
    redactor: &DiagnosticRedactor,
) -> Result<EvidenceBundle, FailureReplayError> {
    #[derive(Clone)]
    struct EventInput {
        identity: TestIdentity,
        artifact: ArtifactReference,
        outcome: Outcome,
        duration_ms: u64,
        diagnostic: Option<String>,
    }

    let mut event_inputs = Vec::with_capacity(evidence.events.len());
    let mut combined = String::new();
    let mut diagnostic_indices = Vec::new();
    let mut individual = Vec::new();
    let mut total = 0usize;

    for (index, event) in evidence.events.iter().enumerate() {
        if event.terminal != matches!(event.event_kind, EventKind::Terminal) {
            return Err(FailureReplayError::Evidence(EvidenceError::InvalidField {
                field: "terminal",
                reason: "terminal flag does not match event kind",
            }));
        }
        let identity = sanitize_test_identity(&event.identity, redactor)?;
        let artifact = sanitize_artifact(&event.artifact, redactor)?;
        let diagnostic = if matches!(event.event_kind, EventKind::Terminal) {
            event.diagnostic.clone()
        } else {
            // Start events cannot retain terminal diagnostics. Drop an
            // externally injected value before rebuilding the valid pair.
            None
        };
        if matches!(event.event_kind, EventKind::Terminal)
            && let Some(raw) = event.diagnostic.as_deref()
        {
            total = total
                .checked_add(raw.len())
                .ok_or(FailureReplayError::Evidence(EvidenceError::InvalidField {
                    field: "diagnostic",
                    reason: "diagnostic byte accounting overflow",
                }))?;
            if total > MAX_EVIDENCE_BYTES {
                return Err(FailureReplayError::Evidence(EvidenceError::InvalidField {
                    field: "diagnostic",
                    reason: "combined persisted diagnostics exceed the one-MiB evidence bound",
                }));
            }
            combined.push_str(raw);
            diagnostic_indices.push(index);
            individual.push(redactor.sanitize(raw).text);
        }
        event_inputs.push(EventInput {
            identity,
            artifact,
            outcome: event.outcome,
            duration_ms: event.duration_ms,
            diagnostic,
        });
    }

    if !diagnostic_indices.is_empty() {
        // Sanitize the complete deterministic diagnostic stream as well as
        // each field. The complete pass closes the gap where a credential is
        // split at an event/chunk boundary.
        let combined = sanitize_channels(redactor, combined.as_bytes(), &[], MAX_EVIDENCE_BYTES)?
            .stdout
            .text;
        let individual_bytes = individual.iter().map(String::as_str).collect::<String>();
        if individual_bytes == combined {
            for (index, diagnostic) in diagnostic_indices.iter().copied().zip(individual) {
                event_inputs[index].diagnostic = Some(diagnostic);
            }
        } else {
            // A cross-field replacement can change byte offsets. Keep the
            // complete sanitized stream on the first deterministic terminal
            // event and clear the remaining copies.
            let first = diagnostic_indices[0];
            event_inputs[first].diagnostic = Some(combined);
            for index in diagnostic_indices.into_iter().skip(1) {
                event_inputs[index].diagnostic = None;
            }
        }
    }

    // Rebuild through the public recorder. This recomputes all event IDs,
    // correlation IDs, peer accounting, and the terminal projection instead
    // of retaining attacker-controlled control fields from the input bundle.
    let recorder = EventRecorder::new(redactor.clone());
    let mut pairs = BTreeMap::<String, (Option<EventInput>, Option<EventInput>)>::new();
    for (event, source) in evidence.events.iter().zip(event_inputs) {
        let key = event.identity.correlation_id();
        let pair = pairs.entry(key).or_default();
        match event.event_kind {
            EventKind::Start => {
                if pair.0.replace(source).is_some() {
                    return Err(FailureReplayError::Evidence(EvidenceError::DuplicateCase(
                        event.identity.case_id.clone(),
                    )));
                }
            }
            EventKind::Terminal => {
                if pair.1.replace(source).is_some() {
                    return Err(FailureReplayError::Evidence(
                        EvidenceError::DuplicateTerminal(event.identity.case_id.clone()),
                    ));
                }
            }
        }
    }
    if pairs.is_empty() {
        return Err(FailureReplayError::Evidence(EvidenceError::InvalidField {
            field: "events",
            reason: "evidence must contain at least one event",
        }));
    }
    for (_, (start, terminal)) in pairs {
        let start = start.ok_or(FailureReplayError::Evidence(EvidenceError::InvalidField {
            field: "events",
            reason: "every terminal must have a start event",
        }))?;
        let terminal =
            terminal.ok_or(FailureReplayError::Evidence(EvidenceError::InvalidField {
                field: "events",
                reason: "every start must have a terminal event",
            }))?;
        if start.identity != terminal.identity {
            return Err(FailureReplayError::Evidence(EvidenceError::InvalidField {
                field: "identity",
                reason: "start and terminal identities must match",
            }));
        }
        if start.artifact != terminal.artifact {
            return Err(FailureReplayError::Evidence(EvidenceError::InvalidField {
                field: "artifact",
                reason: "start and terminal artifact references must match",
            }));
        }
        let mut step = recorder.start(start.identity, start.artifact)?;
        step.finish_with_duration(
            terminal.outcome,
            terminal.duration_ms,
            terminal.diagnostic.as_deref(),
        )?;
    }
    Ok(recorder.finalize()?)
}

fn sanitize_test_identity(
    identity: &TestIdentity,
    redactor: &DiagnosticRedactor,
) -> Result<TestIdentity, FailureReplayError> {
    let source_plan_config = SourcePlanConfig::new(
        sanitize_text(
            redactor,
            "evidence.identity.source",
            &identity.source_plan_config.source,
        )?,
        sanitize_text(
            redactor,
            "evidence.identity.plan",
            &identity.source_plan_config.plan,
        )?,
        sanitize_text(
            redactor,
            "evidence.identity.config",
            &identity.source_plan_config.config,
        )?,
    )?;
    Ok(TestIdentity::new(
        sanitize_text(redactor, "evidence.identity.case_id", &identity.case_id)?,
        sanitize_text(redactor, "evidence.identity.suite", &identity.suite)?,
        sanitize_text(
            redactor,
            "evidence.identity.repository",
            &identity.repository,
        )?,
        sanitize_text(redactor, "evidence.identity.stage", &identity.stage)?,
        source_plan_config,
        identity.attempt,
        identity.seed,
        sanitize_text(redactor, "evidence.identity.command", &identity.command)?,
    )?)
}

fn sanitize_artifact(
    artifact: &ArtifactReference,
    redactor: &DiagnosticRedactor,
) -> Result<ArtifactReference, FailureReplayError> {
    match (&artifact.path, &artifact.replay_id) {
        (None, None) => Ok(ArtifactReference::none()),
        (Some(path), Some(replay_id)) => Ok(ArtifactReference::new(
            Path::new(&sanitize_text(redactor, "evidence.artifact.path", path)?),
            sanitize_text(redactor, "evidence.artifact.replay_id", replay_id)?,
        )?),
        _ => Err(FailureReplayError::Evidence(EvidenceError::InvalidField {
            field: "artifact",
            reason: "path and replay_id must be set together",
        })),
    }
}

fn observed_outcome(evidence: &EvidenceBundle) -> Outcome {
    evidence.projection.outcome
}

fn class_matches(class: FailureClass, evidence: &EvidenceBundle) -> bool {
    match class {
        FailureClass::HarnessFailure => evidence.projection.harness_failures > 0,
        FailureClass::ProductFailure => evidence.projection.failed > 0,
        FailureClass::UnsupportedCapability => evidence.projection.skipped > 0,
    }
}

fn sanitize_replayability(
    replayability: Replayability,
    redactor: &DiagnosticRedactor,
) -> Result<Replayability, FailureReplayError> {
    match replayability {
        Replayability::Replayable { command } => Ok(Replayability::Replayable {
            command: command.sanitize(redactor)?,
        }),
        Replayability::NonReplayable { reason } => Ok(Replayability::NonReplayable { reason }),
    }
}

fn checked_text(field: &'static str, value: &str) -> Result<(), FailureReplayError> {
    if value.is_empty() {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "value must not be empty",
        });
    }
    if value.len() > MAX_FIELD_BYTES {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "value exceeds the bounded field size",
        });
    }
    if value.chars().any(char::is_control) {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "control characters are not allowed",
        });
    }
    if contains_absolute_user_path(value) {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "absolute user paths are not allowed in replay data",
        });
    }
    Ok(())
}

fn checked_text_allow_empty(field: &'static str, value: &str) -> Result<(), FailureReplayError> {
    if value.len() > MAX_FIELD_BYTES {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "value exceeds the bounded field size",
        });
    }
    if value.chars().any(char::is_control) {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "control characters are not allowed",
        });
    }
    if contains_absolute_user_path(value) {
        return Err(FailureReplayError::InvalidField {
            field,
            reason: "absolute user paths are not allowed in replay data",
        });
    }
    Ok(())
}

fn contains_absolute_user_path(value: &str) -> bool {
    ["/home/", "/Users/", "\\Users\\", "HOME=", "USERPROFILE="]
        .iter()
        .any(|marker| value.contains(marker))
}

fn sanitize_text(
    redactor: &DiagnosticRedactor,
    field: &'static str,
    value: &str,
) -> Result<String, FailureReplayError> {
    if value.contains(REDACTED_MARKER) {
        checked_text(field, value)?;
        return Ok(value.to_owned());
    }
    let value = redactor.sanitize(value).text;
    checked_text(field, &value)?;
    Ok(value)
}

fn sanitize_text_allow_empty(
    redactor: &DiagnosticRedactor,
    field: &'static str,
    value: &str,
) -> Result<String, FailureReplayError> {
    if value.contains(REDACTED_MARKER) {
        checked_text_allow_empty(field, value)?;
        return Ok(value.to_owned());
    }
    let value = redactor.sanitize(value).text;
    checked_text_allow_empty(field, &value)?;
    Ok(value)
}

fn sanitize_config_value(
    redactor: &DiagnosticRedactor,
    key: &str,
    value: &str,
) -> Result<String, FailureReplayError> {
    if value.contains(REDACTED_MARKER) {
        checked_text_allow_empty("command.config.value", value)?;
        return Ok(value.to_owned());
    }
    let combined = format!("{key}={value}");
    let sanitized = redactor.sanitize(&combined).text;
    let prefix = format!("{key}=");
    let value = sanitized
        .strip_prefix(&prefix)
        .unwrap_or(sanitized.as_str())
        .to_owned();
    checked_text_allow_empty("command.config.value", &value)?;
    Ok(value)
}

fn safe_relative_text(
    redactor: &DiagnosticRedactor,
    field: &'static str,
    value: &str,
) -> Result<String, FailureReplayError> {
    let value = sanitize_text(redactor, field, value)?;
    checked_relative_path(&value)
}

fn checked_relative_path(value: &str) -> Result<String, FailureReplayError> {
    if value.is_empty() || Path::new(value).is_absolute() {
        return Err(FailureReplayError::InvalidPath(value.to_owned()));
    }
    let mut saw_component = false;
    for component in Path::new(value).components() {
        match component {
            Component::Normal(part) if !part.to_string_lossy().chars().any(char::is_control) => {
                saw_component = true;
            }
            _ => return Err(FailureReplayError::InvalidPath(value.to_owned())),
        }
    }
    if !saw_component {
        return Err(FailureReplayError::InvalidPath(value.to_owned()));
    }
    Ok(value.to_owned())
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:-=@+%,".contains(&byte))
    {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn render_command(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_recipe_quotes_shell_metacharacters() {
        let command = ReplayCommand::new("cargo", ["test;rm", "x y"]);
        assert_eq!(command.render(), "cargo 'test;rm' 'x y'");
    }

    #[test]
    fn relative_paths_reject_root_and_parent_components() {
        assert!(checked_relative_path("/tmp/replay.json").is_err());
        assert!(checked_relative_path("../replay.json").is_err());
        assert!(checked_relative_path("artifacts/replay.json").is_ok());
    }
}
