//! Typed run/repository journal event schema with monotonic checkpoints.
//!
//! This module owns the versioned event model only: exact JSONL rendering and
//! strict parsing, allowed transitions, checkpoint monotonicity, and
//! evidence bounds/redaction policy.  The journal writer and crash-safe
//! replay live in the persistence substrate; the invocation-boundary first
//! record is owned by `run_record`.

#![allow(dead_code)]

use std::{error::Error, fmt};

#[cfg(test)]
mod event_tests;

/// The only supported journal event version.  Unknown required versions fail.
pub const EVENT_VERSION: u8 = 1;

/// The first line of a run record is the invocation intent at checkpoint 0.
pub const INVOCATION_CHECKPOINT: Checkpoint = 0;

/// Monotonic per-run event sequence number.
pub type Checkpoint = u64;

/// Maximum accepted evidence path length and referenced byte bound.
pub const MAX_EVIDENCE_PATH_BYTES: usize = 4096;
pub const MAX_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;

/// Run lifecycle stages in declaration order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStage {
    Invocation,
    Preflight,
    Admission,
    Synchronization,
    Verification,
    GitDelivery,
    Finalization,
}

impl RunStage {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Invocation => "invocation",
            Self::Preflight => "preflight",
            Self::Admission => "admission",
            Self::Synchronization => "synchronization",
            Self::Verification => "verification",
            Self::GitDelivery => "git_delivery",
            Self::Finalization => "finalization",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EventError> {
        match value {
            "invocation" => Ok(Self::Invocation),
            "preflight" => Ok(Self::Preflight),
            "admission" => Ok(Self::Admission),
            "synchronization" => Ok(Self::Synchronization),
            "verification" => Ok(Self::Verification),
            "git_delivery" => Ok(Self::GitDelivery),
            "finalization" => Ok(Self::Finalization),
            other => Err(EventError::UnknownStage(other.to_owned())),
        }
    }
}

/// Repository operation kinds that can carry intents and results.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Synchronize,
    Verify,
    Commit,
    Push,
    Repair,
}

impl Operation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Synchronize => "sync",
            Self::Verify => "verify",
            Self::Commit => "commit",
            Self::Push => "push",
            Self::Repair => "repair",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EventError> {
        match value {
            "sync" => Ok(Self::Synchronize),
            "verify" => Ok(Self::Verify),
            "commit" => Ok(Self::Commit),
            "push" => Ok(Self::Push),
            "repair" => Ok(Self::Repair),
            other => Err(EventError::UnknownOperation(other.to_owned())),
        }
    }
}

/// Terminal and per-repository outcomes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Success,
    Failed,
    Cancelled,
}

impl Outcome {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EventError> {
        match value {
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(EventError::UnknownOutcome(other.to_owned())),
        }
    }
}

/// Evidence reference kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceKind {
    Process,
    Git,
    Agent,
}

impl EvidenceKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Git => "git",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Result<Self, EventError> {
        match value {
            "process" => Ok(Self::Process),
            "git" => Ok(Self::Git),
            "agent" => Ok(Self::Agent),
            other => Err(EventError::UnknownEvidenceKind(other.to_owned())),
        }
    }
}

/// Path-only evidence reference subject to bounds and redaction policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub path: String,
    pub bytes: u64,
}

impl EvidenceRef {
    pub fn new(
        kind: EvidenceKind,
        path: impl Into<String>,
        bytes: u64,
    ) -> Result<Self, EventError> {
        let path = path.into();
        if path.is_empty() || path.len() > MAX_EVIDENCE_PATH_BYTES {
            return Err(EventError::EvidenceBounds { path });
        }
        if bytes > MAX_EVIDENCE_BYTES {
            return Err(EventError::EvidenceBounds { path });
        }
        // Control characters, newlines, ANSI escapes, and JSON metacharacters
        // must stay inert: the exact JSONL render must never be injectable
        // through an evidence reference.
        if path
            .bytes()
            .any(|byte| byte < 0x20 || byte == 0x7f || byte == b'"' || byte == b'\\')
        {
            return Err(EventError::UnsafeEvidencePath { path });
        }
        let lowered = path.to_ascii_lowercase();
        for marker in [
            "token",
            "secret",
            "password",
            "passwd",
            "api_key",
            "apikey",
            "credential",
            "authorization",
            "private_key",
            "bearer",
        ] {
            if lowered.contains(marker) {
                return Err(EventError::SecretBearingEvidence { path });
            }
        }
        Ok(Self { kind, path, bytes })
    }
}

/// Every journal event after the invocation intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalEvent {
    RunIntent {
        checkpoint: Checkpoint,
        run_id: String,
        stage: RunStage,
    },
    RepositoryIntent {
        checkpoint: Checkpoint,
        run_id: String,
        repository_id: String,
        operation: Operation,
        attempt: u8,
    },
    RepositoryResult {
        checkpoint: Checkpoint,
        run_id: String,
        repository_id: String,
        operation: Operation,
        attempt: u8,
        outcome: Outcome,
    },
    SnapshotRecorded {
        checkpoint: Checkpoint,
        run_id: String,
        repository_id: String,
        snapshot_id: String,
        revision: String,
    },
    Evidence {
        checkpoint: Checkpoint,
        run_id: String,
        repository_id: Option<String>,
        evidence: EvidenceRef,
        stage: Option<&'static str>,
    },
    Cancelled {
        checkpoint: Checkpoint,
        run_id: String,
    },
    Terminal {
        checkpoint: Checkpoint,
        run_id: String,
        outcome: Outcome,
    },
}

impl JournalEvent {
    pub fn checkpoint(&self) -> Checkpoint {
        match self {
            Self::RunIntent { checkpoint, .. }
            | Self::RepositoryIntent { checkpoint, .. }
            | Self::RepositoryResult { checkpoint, .. }
            | Self::SnapshotRecorded { checkpoint, .. }
            | Self::Evidence { checkpoint, .. }
            | Self::Cancelled { checkpoint, .. }
            | Self::Terminal { checkpoint, .. } => *checkpoint,
        }
    }

    /// Rebuild the event with the writer-assigned checkpoint.  Producers
    /// never choose checkpoints; the single writer owns allocation.
    pub fn with_checkpoint(&self, checkpoint: Checkpoint) -> Self {
        match self {
            Self::RunIntent { run_id, stage, .. } => Self::RunIntent {
                checkpoint,
                run_id: run_id.clone(),
                stage: *stage,
            },
            Self::RepositoryIntent {
                run_id,
                repository_id,
                operation,
                attempt,
                ..
            } => Self::RepositoryIntent {
                checkpoint,
                run_id: run_id.clone(),
                repository_id: repository_id.clone(),
                operation: *operation,
                attempt: *attempt,
            },
            Self::RepositoryResult {
                run_id,
                repository_id,
                operation,
                attempt,
                outcome,
                ..
            } => Self::RepositoryResult {
                checkpoint,
                run_id: run_id.clone(),
                repository_id: repository_id.clone(),
                operation: *operation,
                attempt: *attempt,
                outcome: *outcome,
            },
            Self::SnapshotRecorded {
                run_id,
                repository_id,
                snapshot_id,
                revision,
                ..
            } => Self::SnapshotRecorded {
                checkpoint,
                run_id: run_id.clone(),
                repository_id: repository_id.clone(),
                snapshot_id: snapshot_id.clone(),
                revision: revision.clone(),
            },
            Self::Evidence {
                run_id,
                repository_id,
                evidence,
                stage,
                ..
            } => Self::Evidence {
                checkpoint,
                run_id: run_id.clone(),
                repository_id: repository_id.clone(),
                evidence: evidence.clone(),
                stage: *stage,
            },
            Self::Cancelled { run_id, .. } => Self::Cancelled {
                checkpoint,
                run_id: run_id.clone(),
            },
            Self::Terminal {
                run_id, outcome, ..
            } => Self::Terminal {
                checkpoint,
                run_id: run_id.clone(),
                outcome: *outcome,
            },
        }
    }

    pub fn run_id(&self) -> &str {
        match self {
            Self::RunIntent { run_id, .. }
            | Self::RepositoryIntent { run_id, .. }
            | Self::RepositoryResult { run_id, .. }
            | Self::SnapshotRecorded { run_id, .. }
            | Self::Evidence { run_id, .. }
            | Self::Cancelled { run_id, .. }
            | Self::Terminal { run_id, .. } => run_id,
        }
    }

    /// Exact canonical JSONL rendering, matching the invocation intent line
    /// format produced by the run-record seam.
    pub fn render(&self) -> String {
        let checkpoint = self.checkpoint();
        let run_id = self.run_id();
        let base = |extra: &str| {
            format!(
                "{{\"version\":{EVENT_VERSION},\"checkpoint\":{checkpoint},\"run_id\":\"{run_id}\"{extra}}}\n"
            )
        };
        match self {
            Self::RunIntent { stage, .. } => base(&format!(
                ",\"type\":\"run_intent\",\"stage\":\"{}\",\"status\":\"started\"",
                stage.name()
            )),
            Self::RepositoryIntent {
                repository_id,
                operation,
                attempt,
                ..
            } => base(&format!(
                ",\"type\":\"repository_intent\",\"repository_id\":\"{repository_id}\",\"operation\":\"{}\",\"attempt\":{attempt}",
                operation.name()
            )),
            Self::RepositoryResult {
                repository_id,
                operation,
                attempt,
                outcome,
                ..
            } => base(&format!(
                ",\"type\":\"repository_result\",\"repository_id\":\"{repository_id}\",\"operation\":\"{}\",\"attempt\":{attempt},\"outcome\":\"{}\"",
                operation.name(),
                outcome.name()
            )),
            Self::SnapshotRecorded {
                repository_id,
                snapshot_id,
                revision,
                ..
            } => base(&format!(
                ",\"type\":\"snapshot_recorded\",\"repository_id\":\"{repository_id}\",\"snapshot_id\":\"{snapshot_id}\",\"revision\":\"{revision}\""
            )),
            Self::Evidence {
                repository_id,
                evidence,
                stage,
                ..
            } => {
                let repository = repository_id
                    .as_ref()
                    .map(|id| format!(",\"repository_id\":\"{id}\""))
                    .unwrap_or_default();
                let stage = stage
                    .map(|label| format!(",\"stage\":\"{label}\""))
                    .unwrap_or_default();
                base(&format!(
                    ",\"type\":\"evidence\",\"kind\":\"{}\"{repository}{stage},\"path\":\"{}\",\"bytes\":{}",
                    evidence.kind.name(),
                    evidence.path,
                    evidence.bytes
                ))
            }
            Self::Cancelled { .. } => base(",\"type\":\"cancelled\""),
            Self::Terminal { outcome, .. } => base(&format!(
                ",\"type\":\"terminal\",\"outcome\":\"{}\"",
                outcome.name()
            )),
        }
    }

    /// Strict parse of one canonical JSONL line.  Unknown versions, unknown
    /// types or values, and malformed or missing fields fail closed.
    pub fn parse(line: &str) -> Result<Self, EventError> {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return Err(EventError::Malformed);
        }
        let fields = parse_fields(trimmed)?;
        let version = required_number(&fields, "version")?;
        if version != EVENT_VERSION as u64 {
            return Err(EventError::UnknownVersion(version));
        }
        let checkpoint = required_number(&fields, "checkpoint")?;
        let run_id = required_string(&fields, "run_id")?;
        let event_type = required_string(&fields, "type")?;
        let outcome = |key: &str| -> Result<Outcome, EventError> {
            Outcome::parse(&required_string(&fields, key)?)
        };
        let stage = || RunStage::parse(&required_string(&fields, "stage")?);
        let operation = || Operation::parse(&required_string(&fields, "operation")?);
        let repository_id = || required_string(&fields, "repository_id");
        let attempt = || -> Result<u8, EventError> {
            let value = required_number(&fields, "attempt")?;
            u8::try_from(value).map_err(|_| EventError::Malformed)
        };
        match event_type.as_str() {
            "run_intent" => {
                let stage = stage()?;
                let status = required_string(&fields, "status")?;
                if status != "started" {
                    return Err(EventError::UnknownStatus(status));
                }
                Ok(Self::RunIntent {
                    checkpoint,
                    run_id,
                    stage,
                })
            }
            "repository_intent" => Ok(Self::RepositoryIntent {
                checkpoint,
                run_id,
                repository_id: repository_id()?,
                operation: operation()?,
                attempt: attempt()?,
            }),
            "repository_result" => Ok(Self::RepositoryResult {
                checkpoint,
                run_id,
                repository_id: repository_id()?,
                operation: operation()?,
                attempt: attempt()?,
                outcome: outcome("outcome")?,
            }),
            "snapshot_recorded" => Ok(Self::SnapshotRecorded {
                checkpoint,
                run_id,
                repository_id: repository_id()?,
                snapshot_id: required_string(&fields, "snapshot_id")?,
                revision: required_string(&fields, "revision")?,
            }),
            "evidence" => {
                let kind = EvidenceKind::parse(&required_string(&fields, "kind")?)?;
                let path = required_string(&fields, "path")?;
                let bytes = required_number(&fields, "bytes")?;
                let repository_id = field(&fields, "repository_id").map(str::to_owned);
                let stage = field(&fields, "stage").map(str::to_owned);
                let stage = match stage.as_deref() {
                    None | Some("compare" | "write" | "publish" | "cleanup" | "admission") => stage,
                    Some(other) => {
                        return Err(EventError::UnknownStage(other.to_owned()));
                    }
                };
                let stage: Option<&'static str> = stage.as_deref().and_then(static_stage);
                Ok(Self::Evidence {
                    checkpoint,
                    run_id,
                    repository_id,
                    evidence: EvidenceRef::new(kind, path, bytes)?,
                    stage,
                })
            }
            "cancelled" => Ok(Self::Cancelled { checkpoint, run_id }),
            "terminal" => Ok(Self::Terminal {
                checkpoint,
                run_id,
                outcome: outcome("outcome")?,
            }),
            other => Err(EventError::UnknownType(other.to_owned())),
        }
    }
}

/// Map a validated evidence stage label to its static form.
fn static_stage(label: &str) -> Option<&'static str> {
    match label {
        "compare" => Some("compare"),
        "admission" => Some("admission"),
        "write" => Some("write"),
        "publish" => Some("publish"),
        "cleanup" => Some("cleanup"),
        _ => None,
    }
}

/// Strict incremental transition validator for one run's event stream.
#[derive(Debug, Default)]
pub struct EventLog {
    started: bool,
    terminal: bool,
    cancelled: bool,
    last_checkpoint: Option<Checkpoint>,
    intents: Vec<(String, Operation, u8)>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the run reached a terminal or cancelled state.
    pub(crate) fn is_terminal(&self) -> bool {
        self.terminal || self.cancelled
    }

    /// Accept one event, enforcing version, monotonic checkpoints, and the
    /// run/repository transition rules.  On error the log state is unchanged.
    pub fn record(&mut self, event: &JournalEvent) -> Result<(), EventError> {
        let checkpoint = event.checkpoint();
        if let Some(previous) = self.last_checkpoint {
            if checkpoint <= previous {
                return Err(EventError::NonMonotonicCheckpoint {
                    expected_after: previous,
                    actual: checkpoint,
                });
            }
        }
        match event {
            JournalEvent::RunIntent { stage, .. } => {
                if self.started {
                    return Err(EventError::InvalidTransition {
                        from: "run started",
                        to: "duplicate run intent",
                    });
                }
                if *stage != RunStage::Invocation {
                    return Err(EventError::InvalidTransition {
                        from: "run intent",
                        to: "non-invocation stage",
                    });
                }
                self.started = true;
            }
            JournalEvent::Cancelled { .. } => {
                if !self.started {
                    return Err(EventError::InvalidTransition {
                        from: "no run",
                        to: "cancellation",
                    });
                }
                if self.cancelled || self.terminal {
                    return Err(EventError::InvalidTransition {
                        from: "already terminal",
                        to: "cancellation",
                    });
                }
                self.cancelled = true;
            }
            JournalEvent::Terminal { outcome, .. } => {
                if !self.started {
                    return Err(EventError::InvalidTransition {
                        from: "no run",
                        to: "terminal",
                    });
                }
                if self.terminal || self.cancelled {
                    return Err(EventError::InvalidTransition {
                        from: "already terminal",
                        to: "terminal",
                    });
                }
                if *outcome == Outcome::Cancelled {
                    return Err(EventError::InvalidTransition {
                        from: "terminal",
                        to: "cancelled outcome",
                    });
                }
                self.terminal = true;
            }
            JournalEvent::RepositoryIntent {
                repository_id,
                operation,
                attempt,
                ..
            } => {
                require_running(self, "repository intent")?;
                let key = (repository_id.clone(), *operation, *attempt);
                if self.intents.contains(&key) {
                    return Err(EventError::InvalidTransition {
                        from: "open repository intent",
                        to: "duplicate intent without result",
                    });
                }
                self.intents.push(key);
            }
            JournalEvent::RepositoryResult {
                repository_id,
                operation,
                attempt,
                outcome,
                ..
            } => {
                require_running(self, "repository result")?;
                let key = (repository_id.clone(), *operation, *attempt);
                let Some(index) = self.intents.iter().position(|candidate| *candidate == key)
                else {
                    return Err(EventError::InvalidTransition {
                        from: "repository result",
                        to: "no matching intent",
                    });
                };
                self.intents.remove(index);
                if *outcome == Outcome::Cancelled {
                    self.cancelled = true;
                }
            }
            JournalEvent::SnapshotRecorded { .. } | JournalEvent::Evidence { .. } => {
                require_running(self, "run state event")?;
            }
        }
        self.last_checkpoint = Some(checkpoint);
        Ok(())
    }
}

fn require_running(log: &EventLog, what: &'static str) -> Result<(), EventError> {
    if !log.started {
        return Err(EventError::InvalidTransition {
            from: "no run",
            to: what,
        });
    }
    if log.terminal || log.cancelled {
        return Err(EventError::InvalidTransition {
            from: "terminal run",
            to: what,
        });
    }
    Ok(())
}

/// Parse a flat JSON object into fields, preserving order.  Nested objects
/// and arrays are rejected; the journal schema is flat by contract.
fn parse_fields(line: &str) -> Result<Vec<(String, String)>, EventError> {
    let body = line
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .ok_or(EventError::Malformed)?;
    let mut fields = Vec::new();
    let mut rest = body.trim();
    while !rest.is_empty() {
        let comma = rest.strip_prefix(',').unwrap_or(rest);
        rest = comma.trim_start();
        let Some(colon) = rest.find(':') else {
            return Err(EventError::Malformed);
        };
        let key = rest[..colon].trim();
        let key = unquote(key).map_err(|_| EventError::Malformed)?;
        let value_rest = rest[colon + 1..].trim_start();
        let (value, remainder) = if value_rest.starts_with('"') {
            let mut escaped = false;
            let mut end = None;
            for (index, byte) in value_rest.as_bytes().iter().enumerate().skip(1) {
                if escaped {
                    escaped = false;
                    continue;
                }
                if *byte == b'\\' {
                    escaped = true;
                    continue;
                }
                if *byte == b'"' {
                    end = Some(index);
                    break;
                }
            }
            let Some(end) = end else {
                return Err(EventError::Malformed);
            };
            let raw = &value_rest[..=end];
            let value = unquote(raw).map_err(|_| EventError::Malformed)?;
            (value.to_owned(), &value_rest[end + 1..])
        } else {
            let end = value_rest
                .find(',')
                .unwrap_or(value_rest.len())
                .min(value_rest.find('}').unwrap_or(value_rest.len()));
            (value_rest[..end].trim().to_owned(), &value_rest[end..])
        };
        if value.contains('{') || value.contains('}') {
            return Err(EventError::Malformed);
        }
        fields.push((key.to_owned(), value));
        rest = remainder.trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        }
    }
    Ok(fields)
}

fn unquote(value: &str) -> Result<&str, ()> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        Ok(&value[1..value.len() - 1])
    } else {
        Err(())
    }
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
}

fn required_string(fields: &[(String, String)], key: &str) -> Result<String, EventError> {
    field(fields, key)
        .map(str::to_owned)
        .ok_or_else(|| EventError::MissingField(key.to_owned()))
}

fn required_number(fields: &[(String, String)], key: &str) -> Result<u64, EventError> {
    let value = required_string(fields, key)?;
    value.parse::<u64>().map_err(|_| EventError::Malformed)
}

/// Schema and transition failures.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EventError {
    UnknownVersion(u64),
    UnknownType(String),
    UnknownStage(String),
    UnknownStatus(String),
    UnknownOperation(String),
    UnknownOutcome(String),
    UnknownEvidenceKind(String),
    MissingField(String),
    Malformed,
    NonMonotonicCheckpoint {
        expected_after: Checkpoint,
        actual: Checkpoint,
    },
    InvalidTransition {
        from: &'static str,
        to: &'static str,
    },
    EvidenceBounds {
        path: String,
    },
    SecretBearingEvidence {
        path: String,
    },
    UnsafeEvidencePath {
        path: String,
    },
}

impl fmt::Display for EventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVersion(version) => {
                write!(formatter, "unsupported journal event version {version}")
            }
            Self::UnknownType(value) => write!(formatter, "unknown journal event type {value:?}"),
            Self::UnknownStage(value) => write!(formatter, "unknown run stage {value:?}"),
            Self::UnknownStatus(value) => write!(formatter, "unknown run status {value:?}"),
            Self::UnknownOperation(value) => {
                write!(formatter, "unknown repository operation {value:?}")
            }
            Self::UnknownOutcome(value) => write!(formatter, "unknown outcome {value:?}"),
            Self::UnknownEvidenceKind(value) => {
                write!(formatter, "unknown evidence kind {value:?}")
            }
            Self::MissingField(field) => write!(formatter, "journal event is missing {field:?}"),
            Self::Malformed => formatter.write_str("malformed journal event line"),
            Self::NonMonotonicCheckpoint {
                expected_after,
                actual,
            } => write!(
                formatter,
                "checkpoint {actual} is not after the previous checkpoint {expected_after}"
            ),
            Self::InvalidTransition { from, to } => {
                write!(formatter, "invalid journal transition: {from} -> {to}")
            }
            Self::EvidenceBounds { path } => {
                write!(formatter, "evidence reference exceeds bounds: {path}")
            }
            Self::SecretBearingEvidence { path } => {
                write!(formatter, "evidence reference may carry secrets: {path}")
            }
            Self::UnsafeEvidencePath { path } => {
                write!(
                    formatter,
                    "evidence reference contains unsafe characters: {path}"
                )
            }
        }
    }
}
impl Error for EventError {}
