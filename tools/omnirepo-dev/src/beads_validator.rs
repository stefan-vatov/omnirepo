//! Read-only validation of the tracked Beads decision workflow.
//!
//! This module deliberately has no dependency on `br` or `bv`.  It reads the
//! tracked JSONL export and returns typed findings.  The command line layer
//! renders those findings as text or JSON; it never changes the export.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

pub const REPORT_SCHEMA: &str = "omnirepo.decision-validation.v1";
pub const TEXT_REPORT_SCHEMA: &str = "decision-workflow-text.v1";
pub const DEFAULT_TRACKED_JSONL: &str = ".beads/issues.jsonl";
pub const MAX_FINDINGS: usize = 64;
pub const MAX_DIAGNOSTIC_TEXT: usize = 256;

const DECISION_NEEDED: &str = "decision-needed";
const HUMAN_INPUT: &str = "human-input";

/// The status values accepted by the tracked Beads export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueStatus {
    Open,
    Decision,
    InProgress,
    Blocked,
    Deferred,
    Closed,
    Tombstone,
}

impl IssueStatus {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "open" => Self::Open,
            "decision" => Self::Decision,
            "in_progress" => Self::InProgress,
            "blocked" => Self::Blocked,
            "deferred" => Self::Deferred,
            "closed" => Self::Closed,
            "tombstone" => Self::Tombstone,
            _ => return None,
        })
    }
}

/// A non-empty issue identifier from the tracked export.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct IssueId(String);

impl IssueId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A label from a valid labels array.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label(String);

impl Label {
    fn new(value: &str) -> Self {
        Self(value.to_owned())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable machine-readable finding categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingCode {
    BlankJsonlRecord,
    MalformedJsonRecord,
    RecordNotObject,
    MissingIssueId,
    UnknownStatus,
    LabelsNotArray,
    LabelNotString,
    DuplicateLabel,
    DuplicateIssueId,
    ActiveDecisionLabelsMissing,
    ClosedDecisionLabelsMissing,
    ClosedDecisionProvenanceMissing,
    DecisionLabelsRequireDecision,
    TrackedJsonlMissing,
    TrackedJsonlEmpty,
    TrackedJsonlUnreadable,
    TrackedJsonlInvalidUtf8,
}

impl FindingCode {
    fn message(self) -> &'static str {
        match self {
            Self::BlankJsonlRecord => "blank JSONL record",
            Self::MalformedJsonRecord => "malformed JSON record",
            Self::RecordNotObject => "record must be a JSON object",
            Self::MissingIssueId => "id must be a non-empty string",
            Self::UnknownStatus => {
                "status must be one of open, decision, in_progress, blocked, deferred, closed, tombstone"
            }
            Self::LabelsNotArray => "labels must be an array when present",
            Self::LabelNotString => "labels must contain only strings",
            Self::DuplicateLabel => "labels must not contain duplicates",
            Self::DuplicateIssueId => "duplicate issue ID",
            Self::ActiveDecisionLabelsMissing => {
                "active decision requires status=decision with both decision-needed and human-input labels"
            }
            Self::ClosedDecisionLabelsMissing => {
                "closed decision requires both decision-needed and human-input labels"
            }
            Self::ClosedDecisionProvenanceMissing => "closed decision is missing provenance fields",
            Self::DecisionLabelsRequireDecision => {
                "decision labels require status=decision; non-closed work cannot carry decision-needed or human-input"
            }
            Self::TrackedJsonlMissing => "tracked JSONL is missing",
            Self::TrackedJsonlEmpty => "tracked JSONL is empty",
            Self::TrackedJsonlUnreadable => "tracked JSONL is unreadable",
            Self::TrackedJsonlInvalidUtf8 => "tracked JSONL is not valid UTF-8",
        }
    }
}

impl fmt::Display for FindingCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        serde_json::to_string(self)
            .map_err(|_| fmt::Error)
            .and_then(|value| formatter.write_str(value.trim_matches('"')))
    }
}

/// One bounded, stable diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: FindingCode,
    pub line: Option<usize>,
    pub issue_id: Option<IssueId>,
    pub message: String,
}

impl Finding {
    fn new(code: FindingCode, line: Option<usize>, issue_id: Option<IssueId>) -> Self {
        Self {
            code,
            line,
            issue_id,
            message: code.message().to_owned(),
        }
    }

    fn with_message(
        code: FindingCode,
        line: Option<usize>,
        issue_id: Option<IssueId>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            line,
            issue_id,
            message: bounded_text(&message.into()),
        }
    }
}

/// The machine-readable result of validating one tracked export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub schema: &'static str,
    pub status: ValidationStatus,
    pub path: String,
    pub findings: Vec<Finding>,
    pub truncated: bool,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.status == ValidationStatus::Consistent
    }
}

/// A validation result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    Consistent,
    Invalid,
}

/// Errors encountered while opening the tracked export.
#[derive(Debug)]
pub enum ValidatorError {
    Missing { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
    InvalidUtf8 { path: PathBuf },
}

impl ValidatorError {
    pub fn path(&self) -> &Path {
        match self {
            Self::Missing { path } | Self::Io { path, .. } | Self::InvalidUtf8 { path } => path,
        }
    }

    pub fn into_report(self) -> ValidationReport {
        let path = bounded_text(&self.path().display().to_string());
        let (code, message) = match &self {
            Self::Missing { .. } => (FindingCode::TrackedJsonlMissing, "tracked JSONL is missing"),
            Self::Io { .. } => (
                FindingCode::TrackedJsonlUnreadable,
                "tracked JSONL is unreadable",
            ),
            Self::InvalidUtf8 { .. } => (
                FindingCode::TrackedJsonlInvalidUtf8,
                "tracked JSONL is not valid UTF-8",
            ),
        };
        invalid_report(path, Finding::with_message(code, None, None, message))
    }
}

impl fmt::Display for ValidatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => {
                write!(formatter, "tracked JSONL is missing: {}", path.display())
            }
            Self::Io { path, source } => {
                write!(
                    formatter,
                    "cannot read tracked JSONL {}: {source}",
                    path.display()
                )
            }
            Self::InvalidUtf8 { path } => {
                write!(
                    formatter,
                    "tracked JSONL is not valid UTF-8: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ValidatorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Missing { .. } | Self::InvalidUtf8 { .. } => None,
        }
    }
}

/// Validate the export selected by `BEADS_JSONL`, or the repository default.
pub fn validate_default() -> Result<ValidationReport, ValidatorError> {
    let path = std::env::var_os("BEADS_JSONL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TRACKED_JSONL));
    validate_path(&path)
}

/// Validate one tracked export without invoking tracker executables.
pub fn validate_path(path: &Path) -> Result<ValidationReport, ValidatorError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(ValidatorError::Missing {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(ValidatorError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let contents = String::from_utf8(bytes).map_err(|_| ValidatorError::InvalidUtf8 {
        path: path.to_path_buf(),
    })?;
    Ok(validate_contents(path, &contents))
}

/// Validate JSONL text.  The input is never written or normalized.
pub fn validate_contents(path: &Path, contents: &str) -> ValidationReport {
    let path = bounded_text(&path.display().to_string());
    if contents.is_empty() {
        return invalid_report(
            path,
            Finding::with_message(
                FindingCode::TrackedJsonlEmpty,
                None,
                None,
                FindingCode::TrackedJsonlEmpty.message(),
            ),
        );
    }

    let mut report = ValidationReport {
        schema: REPORT_SCHEMA,
        status: ValidationStatus::Consistent,
        path,
        findings: Vec::new(),
        truncated: false,
    };
    let mut seen_ids: HashMap<IssueId, usize> = HashMap::new();

    for (line_index, raw_line) in contents.lines().enumerate() {
        let line = line_index + 1;
        if raw_line.is_empty() {
            push_finding(
                &mut report,
                Finding::new(FindingCode::BlankJsonlRecord, Some(line), None),
            );
            continue;
        }

        let value = match serde_json::from_str::<Value>(raw_line) {
            Ok(value) => value,
            Err(_) => {
                push_finding(
                    &mut report,
                    Finding::new(FindingCode::MalformedJsonRecord, Some(line), None),
                );
                continue;
            }
        };
        let Some(object) = value.as_object() else {
            push_finding(
                &mut report,
                Finding::new(FindingCode::RecordNotObject, Some(line), None),
            );
            continue;
        };

        let issue_id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| IssueId(value.to_owned()));
        if issue_id.is_none() {
            push_finding(
                &mut report,
                Finding::new(FindingCode::MissingIssueId, Some(line), None),
            );
        } else if let Some(issue_id) = issue_id.as_ref() {
            if seen_ids.insert(issue_id.clone(), line).is_some() {
                push_finding(
                    &mut report,
                    Finding::new(
                        FindingCode::DuplicateIssueId,
                        Some(line),
                        Some(issue_id.clone()),
                    ),
                );
            }
        }

        let status = object
            .get("status")
            .and_then(Value::as_str)
            .and_then(IssueStatus::parse);
        if status.is_none() {
            push_finding(
                &mut report,
                Finding::new(FindingCode::UnknownStatus, Some(line), issue_id.clone()),
            );
        }

        let (labels, labels_valid) = match object.get("labels") {
            None => (Vec::new(), true),
            Some(value) => match value.as_array() {
                None => {
                    push_finding(
                        &mut report,
                        Finding::new(FindingCode::LabelsNotArray, Some(line), issue_id.clone()),
                    );
                    (Vec::new(), false)
                }
                Some(values) => {
                    let duplicate = values
                        .iter()
                        .enumerate()
                        .any(|(index, value)| values[..index].contains(value));
                    if duplicate {
                        push_finding(
                            &mut report,
                            Finding::new(FindingCode::DuplicateLabel, Some(line), issue_id.clone()),
                        );
                    }
                    let labels = values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(Label::new)
                        .collect::<Vec<_>>();
                    let all_strings = values.iter().all(Value::is_string);
                    if !all_strings {
                        push_finding(
                            &mut report,
                            Finding::new(FindingCode::LabelNotString, Some(line), issue_id.clone()),
                        );
                    }
                    (labels, all_strings)
                }
            },
        };

        let Some(status) = status else {
            continue;
        };
        if !labels_valid {
            continue;
        }

        let has_decision_needed = labels.iter().any(|label| label.as_str() == DECISION_NEEDED);
        let has_human_input = labels.iter().any(|label| label.as_str() == HUMAN_INPUT);
        let has_decision_marker = has_decision_needed || has_human_input;
        match status {
            IssueStatus::Decision if !(has_decision_needed && has_human_input) => {
                push_finding(
                    &mut report,
                    Finding::new(
                        FindingCode::ActiveDecisionLabelsMissing,
                        Some(line),
                        issue_id.clone(),
                    ),
                );
            }
            IssueStatus::Decision => {}
            IssueStatus::Closed if has_decision_marker => {
                if !(has_decision_needed && has_human_input) {
                    push_finding(
                        &mut report,
                        Finding::new(
                            FindingCode::ClosedDecisionLabelsMissing,
                            Some(line),
                            issue_id.clone(),
                        ),
                    );
                } else {
                    let missing = ["created_at", "created_by", "closed_at", "close_reason"]
                        .into_iter()
                        .filter(|field| {
                            object
                                .get(*field)
                                .and_then(Value::as_str)
                                .map(str::is_empty)
                                .unwrap_or(true)
                        })
                        .collect::<Vec<_>>();
                    if !missing.is_empty() {
                        let message = format!(
                            "{}: {}",
                            FindingCode::ClosedDecisionProvenanceMissing.message(),
                            missing.join(", ")
                        );
                        push_finding(
                            &mut report,
                            Finding::with_message(
                                FindingCode::ClosedDecisionProvenanceMissing,
                                Some(line),
                                issue_id.clone(),
                                message,
                            ),
                        );
                    }
                }
            }
            IssueStatus::Closed => {}
            _ if has_decision_marker => {
                push_finding(
                    &mut report,
                    Finding::new(
                        FindingCode::DecisionLabelsRequireDecision,
                        Some(line),
                        issue_id.clone(),
                    ),
                );
            }
            _ => {}
        }
    }

    if !report.findings.is_empty() {
        report.status = ValidationStatus::Invalid;
    }
    report
}

fn invalid_report(path: String, finding: Finding) -> ValidationReport {
    ValidationReport {
        schema: REPORT_SCHEMA,
        status: ValidationStatus::Invalid,
        path,
        findings: vec![finding],
        truncated: false,
    }
}

fn push_finding(report: &mut ValidationReport, finding: Finding) {
    if report.findings.len() < MAX_FINDINGS {
        report.findings.push(finding);
    } else {
        report.truncated = true;
    }
    report.status = ValidationStatus::Invalid;
}

fn bounded_text(value: &str) -> String {
    if value.len() <= MAX_DIAGNOSTIC_TEXT {
        return value.to_owned();
    }
    let limit = MAX_DIAGNOSTIC_TEXT - '…'.len_utf8();
    let mut end = 0;
    for (index, character) in value.char_indices() {
        let next = index + character.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    format!("{}…", &value[..end])
}
