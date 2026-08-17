//! Pure checked planning over canonical Beads snapshots.
//!
//! The planner is deliberately separate from the `br` process boundary.  It
//! accepts ordinary JSON/text values, validates the tracked export, compares
//! canonical `ready` and `scheduler` evidence, and emits the frozen
//! `omnirepo.checked-agent-plan.v1` projection.  It never invokes `bv`, never
//! mutates a tracker, and never makes an owner decision.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::beads_validator::{FindingCode, ValidationReport, ValidationStatus, validate_contents};
use crate::br_adapter::{BrAdapter, BrAdapterError, SourceKind};

/// The checked plan schema consumed by repository tooling and the static
/// Viewer adapter.
pub const PLAN_SCHEMA: &str = "omnirepo.checked-agent-plan.v1";

/// Default tracked state path when the caller does not provide a fixture path.
pub const DEFAULT_TRACKED_JSONL: &str = ".beads/issues.jsonl";

const TRACKED_SOURCE_DESCRIPTION: &str = "br ready + br scheduler + tracked JSONL";
const SUMMARY_NOTE: &str = "Only checked br ready/scheduler records with matching tracked status and labels are autonomous candidates; raw bv is advisory-only.";

const OWNER_DECISION_LABEL: &str = "decision-needed";
const HUMAN_INPUT_LABEL: &str = "human-input";
const CONTAINER_LABEL: &str = "workstream-container";
const ALTERNATE_CONTAINER_LABEL: &str = "container";

/// Inputs used by the pure planner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannerInputs<'a> {
    pub ready_json: &'a str,
    pub scheduler_json: &'a str,
    pub tracked_jsonl: &'a str,
    pub tracked_path: &'a Path,
}

/// A planner with a frozen process adapter and tracked-export path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckedPlanner {
    adapter: BrAdapter,
    tracked_jsonl: PathBuf,
}

impl CheckedPlanner {
    pub fn new(adapter: BrAdapter, tracked_jsonl: impl Into<PathBuf>) -> Self {
        Self {
            adapter,
            tracked_jsonl: tracked_jsonl.into(),
        }
    }

    /// Discover `br`, read both canonical sources, and always return a stable
    /// machine-readable result.  Process-boundary failures are represented as
    /// error reports rather than being silently skipped.
    pub fn run(&self) -> CheckedPlan {
        // Match the repository-owned wrapper's fail-closed ordering: a
        // missing tracked export is rejected before any tracker process is
        // started.
        let tracked = match fs::read(&self.tracked_jsonl) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(contents) => contents,
                Err(_) => {
                    return error_report(
                        "tracked-state-invalid",
                        "tracked JSONL is not valid UTF-8",
                        vec![issue("<tracked>", "tracked-export-invalid")],
                    );
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return error_report(
                    "tracked-export-missing",
                    "tracked Beads JSONL is missing",
                    vec![issue_with(
                        "<tracked>",
                        "tracked-export-missing",
                        [(
                            "path",
                            Value::String(self.tracked_jsonl.display().to_string()),
                        )],
                    )],
                );
            }
            Err(error) => {
                return error_report(
                    "tracked-state-invalid",
                    "tracked Beads JSONL is unreadable",
                    vec![issue_with(
                        "<tracked>",
                        "tracked-export-invalid",
                        [("detail", Value::String(bounded_text(&error.to_string())))],
                    )],
                );
            }
        };

        let ready = match self.adapter.ready() {
            Ok(output) => output,
            Err(error) => return report_for_adapter_error(error),
        };
        let scheduler = match self.adapter.scheduler() {
            Ok(output) => output,
            Err(error) => return report_for_adapter_error(error),
        };
        plan(PlannerInputs {
            ready_json: &ready.stdout,
            scheduler_json: &scheduler.stdout,
            tracked_jsonl: &tracked,
            tracked_path: &self.tracked_jsonl,
        })
    }
}

/// Discover a bounded adapter and create a checked planner for one repository.
pub fn discover(
    repository_root: impl AsRef<Path>,
    tracked_jsonl: impl Into<PathBuf>,
) -> Result<CheckedPlanner, BrAdapterError> {
    Ok(CheckedPlanner::new(
        BrAdapter::discover(repository_root)?,
        tracked_jsonl,
    ))
}

/// Run the pure planner against frozen source strings.
pub fn plan(inputs: PlannerInputs<'_>) -> CheckedPlan {
    let ready = match parse_ready(inputs.ready_json) {
        Ok(value) => value,
        Err(error) => {
            return error_report(
                "canonical-source-malformed",
                "br ready did not return the required JSON array contract",
                vec![issue_with(
                    "<ready>",
                    "canonical-ready-malformed",
                    [("detail", Value::String(bounded_text(&error)))],
                )],
            );
        }
    };
    let scheduler = match parse_scheduler(inputs.scheduler_json) {
        Ok(value) => value,
        Err(error) => {
            return error_report(
                "canonical-source-malformed",
                "br scheduler did not return the required JSON object contract",
                vec![issue_with(
                    "<scheduler>",
                    "canonical-scheduler-malformed",
                    [("detail", Value::String(bounded_text(&error)))],
                )],
            );
        }
    };

    let ready_ids = sorted_ids(ready.iter().map(|item| item.id.as_str()));
    let scheduler_ids = sorted_ids(scheduler.iter().map(|item| item.issue.id.as_str()));
    if ready_ids != scheduler_ids {
        return error_report(
            "planner-disagreement",
            "br ready and br scheduler returned different candidate ID sets",
            vec![issue_with(
                "<set>",
                "scheduler-ready-disagreement",
                [
                    ("ready_ids", Value::Array(string_values(&ready_ids))),
                    ("scheduler_ids", Value::Array(string_values(&scheduler_ids))),
                ],
            )],
        );
    }

    let ready_duplicates = duplicate_ids(ready.iter().map(|item| item.id.as_str()));
    let scheduler_duplicates = duplicate_ids(scheduler.iter().map(|item| item.issue.id.as_str()));
    if !ready_duplicates.is_empty() || !scheduler_duplicates.is_empty() {
        let mut issues = Vec::new();
        for id in ready_duplicates {
            issues.push(issue(&id, "duplicate-ready-id"));
        }
        for id in scheduler_duplicates {
            issues.push(issue(&id, "duplicate-scheduler-id"));
        }
        return error_report(
            "planner-disagreement",
            "br ready or br scheduler returned duplicate candidate IDs",
            issues,
        );
    }

    let validation = validate_contents(inputs.tracked_path, inputs.tracked_jsonl);
    if validation.status != ValidationStatus::Consistent {
        return report_for_validation(validation);
    }
    let tracked = match parse_tracked(inputs.tracked_jsonl) {
        Ok(value) => value,
        Err(error) => {
            return error_report(
                "tracked-state-invalid",
                "tracked JSONL could not be parsed after validation",
                vec![issue_with(
                    "<tracked>",
                    "tracked-export-malformed",
                    [("detail", Value::String(bounded_text(&error)))],
                )],
            );
        }
    };

    let ready_by_id = ready
        .iter()
        .map(|item| (item.id.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let tracked_by_id = tracked
        .iter()
        .map(|item| (item.id.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let mut mismatches = Vec::new();
    for item in &ready {
        let Some(record) = tracked_by_id.get(&item.id) else {
            mismatches.push(issue(&item.id, "tracked-issue-missing"));
            continue;
        };
        if item.status != record.status {
            mismatches.push(issue_with(
                &item.id,
                "tracked-status-disagreement",
                [
                    ("ready_status", Value::String(item.status.clone())),
                    ("tracked_status", Value::String(record.status.clone())),
                ],
            ));
        } else if !labels_equal_unordered(&item.labels, &record.labels) {
            mismatches.push(issue_with(
                &item.id,
                "tracked-label-disagreement",
                [
                    ("ready_labels", Value::Array(string_values(&item.labels))),
                    (
                        "tracked_labels",
                        Value::Array(string_values(&record.labels)),
                    ),
                ],
            ));
        }
    }
    for recommendation in &scheduler {
        let Some(ready_item) = ready_by_id.get(&recommendation.issue.id) else {
            mismatches.push(issue(
                &recommendation.issue.id,
                "scheduler-ready-disagreement",
            ));
            continue;
        };
        if recommendation.issue.status != ready_item.status {
            mismatches.push(issue_with(
                &recommendation.issue.id,
                "scheduler-status-disagreement",
                [
                    ("ready_status", Value::String(ready_item.status.clone())),
                    (
                        "scheduler_status",
                        Value::String(recommendation.issue.status.clone()),
                    ),
                ],
            ));
        } else if !recommendation.issue.labels.is_empty()
            && !labels_equal_unordered(&recommendation.issue.labels, &ready_item.labels)
        {
            mismatches.push(issue_with(
                &recommendation.issue.id,
                "scheduler-label-disagreement",
                [
                    (
                        "ready_labels",
                        Value::Array(string_values(&ready_item.labels)),
                    ),
                    (
                        "scheduler_labels",
                        Value::Array(string_values(&recommendation.issue.labels)),
                    ),
                ],
            ));
        }
    }
    if !mismatches.is_empty() {
        return error_report(
            "tracked-state-disagreement",
            "canonical tracker evidence disagrees with the tracked JSONL decision state",
            mismatches,
        );
    }

    let mut candidates = Vec::new();
    let mut excluded = Vec::new();
    for recommendation in scheduler {
        let mut item = recommendation.issue.with_rank(recommendation.rank);
        if recommendation.issue.status == "open"
            && !recommendation.issue.is_decision_marked()
            && !recommendation.issue.is_container()
        {
            item.insert("reason".to_owned(), Value::String("ready".to_owned()));
            candidates.push(PlanItem { fields: item });
        } else if let Some((reason, blocker)) = recommendation.issue.exclusion() {
            item.insert("reason".to_owned(), Value::String(reason));
            item.insert("blocker".to_owned(), Value::String(blocker));
            excluded.push(PlanItem { fields: item });
        }
    }

    let candidate_ids = candidates
        .iter()
        .filter_map(PlanItem::id)
        .collect::<Vec<_>>();
    let excluded_ids = excluded.iter().filter_map(PlanItem::id).collect::<Vec<_>>();
    let total_blocked = excluded
        .iter()
        .filter(|item| item.fields.get("reason") == Some(&Value::String("blocked".to_owned())))
        .count();
    let plan_items = candidates.clone();
    CheckedPlan::success(
        PlanBody {
            tracks: vec![PlanTrack {
                track_id: "checked-ready".to_owned(),
                items: plan_items,
            }],
            total_actionable: candidates.len(),
            total_blocked,
        },
        candidates,
        excluded,
        Evidence {
            source: TRACKED_SOURCE_DESCRIPTION.to_owned(),
            tracked_jsonl: inputs.tracked_path.display().to_string(),
            ready_ids,
            scheduler_ids,
            raw_bv: "advisory-only".to_owned(),
        },
        Summary {
            actionable_ids: candidate_ids,
            excluded_ids,
            note: SUMMARY_NOTE.to_owned(),
        },
    )
}

/// A stable machine-readable plan result.
#[derive(Clone, Debug, Serialize)]
pub struct CheckedPlan {
    pub schema: &'static str,
    pub status: PlanStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<PlanItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<Vec<PlanItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PlanError>,
}

impl CheckedPlan {
    fn success(
        plan: PlanBody,
        candidates: Vec<PlanItem>,
        excluded: Vec<PlanItem>,
        evidence: Evidence,
        summary: Summary,
    ) -> Self {
        Self {
            schema: PLAN_SCHEMA,
            status: PlanStatus::Ok,
            plan: Some(plan),
            candidates: Some(candidates),
            excluded: Some(excluded),
            evidence: Some(evidence),
            summary: Some(summary),
            error: None,
        }
    }
}

/// The plan cannot run without the owner-machine `br` CLI, which CI
/// cannot install.  A visible skip keeps the gate honest without a false
/// failure: the report still names the missing command, and the exit
/// status is zero so the aggregate gate passes on both the owner machine
/// (where `br` exists and the plan really runs) and CI (where the skip is
/// reported).
pub fn is_missing_required_command(report: &CheckedPlan) -> bool {
    matches!(
        report.error.as_ref(),
        Some(error) if error.code == "required-command-missing"
    )
}

/// Plan status values are part of the v1 output contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    Ok,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanBody {
    pub tracks: Vec<PlanTrack>,
    pub total_actionable: usize,
    pub total_blocked: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanTrack {
    pub track_id: String,
    pub items: Vec<PlanItem>,
}

/// An issue object plus the checked planner's reason fields.
#[derive(Clone, Debug, Serialize)]
pub struct PlanItem {
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl PlanItem {
    fn id(&self) -> Option<String> {
        self.fields
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    pub source: String,
    pub tracked_jsonl: String,
    pub ready_ids: Vec<String>,
    pub scheduler_ids: Vec<String>,
    pub raw_bv: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub actionable_ids: Vec<String>,
    pub excluded_ids: Vec<String>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanError {
    pub code: String,
    pub message: String,
    pub issues: Vec<PlanIssue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanIssue {
    pub id: String,
    pub reason: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IssueView {
    raw: Map<String, Value>,
    id: String,
    status: String,
    labels: Vec<String>,
    issue_type: Option<String>,
}

impl IssueView {
    fn with_rank(&self, rank: Value) -> Map<String, Value> {
        let mut value = self.raw.clone();
        value.insert("rank".to_owned(), rank);
        value
    }

    fn is_decision_marked(&self) -> bool {
        self.labels
            .iter()
            .any(|label| label == OWNER_DECISION_LABEL || label == HUMAN_INPUT_LABEL)
    }

    fn is_container(&self) -> bool {
        self.labels
            .iter()
            .any(|label| label == CONTAINER_LABEL || label == ALTERNATE_CONTAINER_LABEL)
            || self.issue_type.as_deref() == Some("epic")
    }

    fn exclusion(&self) -> Option<(String, String)> {
        if self.status == "decision" {
            Some(("owner-decision".to_owned(), "owner-input".to_owned()))
        } else if self.status == "closed" && self.is_decision_marked() {
            Some((
                "closed-decision-history".to_owned(),
                "historical-owner-decision".to_owned(),
            ))
        } else if self.status == "blocked" {
            Some(("blocked".to_owned(), "dependency-or-policy".to_owned()))
        } else if self.status == "deferred" {
            Some(("deferred".to_owned(), "deferred-by-workflow".to_owned()))
        } else if self.is_container() {
            Some(("container".to_owned(), "container-not-leaf-work".to_owned()))
        } else if self.status != "open" {
            Some((
                "status-not-ready".to_owned(),
                format!("status:{}", self.status),
            ))
        } else if self.is_decision_marked() {
            Some((
                "owner-decision".to_owned(),
                "decision-label-drift".to_owned(),
            ))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SchedulerView {
    issue: IssueView,
    rank: Value,
}

fn parse_ready(source: &str) -> Result<Vec<IssueView>, String> {
    let value = serde_json::from_str::<Value>(source).map_err(|error| error.to_string())?;
    let Value::Array(items) = value else {
        return Err("top-level value is not an array".to_owned());
    };
    items.iter().map(parse_issue).collect()
}

fn parse_scheduler(source: &str) -> Result<Vec<SchedulerView>, String> {
    let value = serde_json::from_str::<Value>(source).map_err(|error| error.to_string())?;
    let Value::Object(object) = value else {
        return Err("top-level value is not an object".to_owned());
    };
    let Some(Value::Array(recommendations)) = object.get("recommendations") else {
        return Err("recommendations is not an array".to_owned());
    };
    recommendations
        .iter()
        .map(|recommendation| {
            let Value::Object(recommendation) = recommendation else {
                return Err("recommendation is not an object".to_owned());
            };
            let Some(issue) = recommendation.get("issue") else {
                return Err("recommendation issue is missing".to_owned());
            };
            Ok(SchedulerView {
                issue: parse_issue(issue)?,
                rank: recommendation.get("rank").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn parse_issue(value: &Value) -> Result<IssueView, String> {
    let Value::Object(raw) = value else {
        return Err("issue is not an object".to_owned());
    };
    let Some(id) = raw.get("id").and_then(Value::as_str) else {
        return Err("issue id is missing or not a string".to_owned());
    };
    if id.is_empty() {
        return Err("issue id is empty".to_owned());
    }
    let Some(status) = raw.get("status").and_then(Value::as_str) else {
        return Err("issue status is missing or not a string".to_owned());
    };
    let Some(labels) = raw.get("labels").and_then(Value::as_array) else {
        return Err("issue labels is missing or not an array".to_owned());
    };
    let mut parsed_labels = Vec::with_capacity(labels.len());
    for label in labels {
        let Some(label) = label.as_str() else {
            return Err("issue labels must contain only strings".to_owned());
        };
        parsed_labels.push(label.to_owned());
    }
    Ok(IssueView {
        raw: raw.clone(),
        id: id.to_owned(),
        status: status.to_owned(),
        labels: parsed_labels,
        issue_type: raw
            .get("issue_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn parse_tracked(source: &str) -> Result<Vec<IssueView>, String> {
    source
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let value = serde_json::from_str::<Value>(line).map_err(|error| error.to_string())?;
            parse_tracked_issue(&value)
        })
        .collect()
}

fn parse_tracked_issue(value: &Value) -> Result<IssueView, String> {
    let Value::Object(raw) = value else {
        return Err("tracked issue is not an object".to_owned());
    };
    let Some(id) = raw.get("id").and_then(Value::as_str) else {
        return Err("tracked issue id is missing or not a string".to_owned());
    };
    let Some(status) = raw.get("status").and_then(Value::as_str) else {
        return Err("tracked issue status is missing or not a string".to_owned());
    };
    let labels = match raw.get("labels") {
        None => Vec::new(),
        Some(Value::Array(labels)) => labels
            .iter()
            .map(|label| {
                label
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "tracked issue labels must contain only strings".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("tracked issue labels are not an array".to_owned()),
    };
    Ok(IssueView {
        raw: raw.clone(),
        id: id.to_owned(),
        status: status.to_owned(),
        labels,
        issue_type: raw
            .get("issue_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn report_for_validation(report: ValidationReport) -> CheckedPlan {
    if report.findings.is_empty() {
        return error_report(
            "tracked-state-invalid",
            "tracked JSONL failed the decision workflow validator",
            vec![issue("<tracked>", "tracked-decision-state")],
        );
    }
    let mut issues = Vec::new();
    for finding in report.findings {
        let id = finding
            .issue_id
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| {
                if matches!(
                    finding.code,
                    FindingCode::MalformedJsonRecord | FindingCode::RecordNotObject
                ) {
                    "<malformed>".to_owned()
                } else {
                    "<tracked>".to_owned()
                }
            });
        let reason = match finding.code {
            FindingCode::ActiveDecisionLabelsMissing
            | FindingCode::ClosedDecisionLabelsMissing
            | FindingCode::ClosedDecisionProvenanceMissing
            | FindingCode::DecisionLabelsRequireDecision
            | FindingCode::TrackedJsonlEmpty => "tracked-decision-state",
            FindingCode::DuplicateIssueId => "duplicate-issue-id",
            FindingCode::MalformedJsonRecord | FindingCode::RecordNotObject => {
                "tracked-record-malformed"
            }
            _ => "tracked-export-invalid",
        };
        issues.push(issue_with(
            &id,
            reason,
            [("detail", Value::String(bounded_text(&finding.message)))],
        ));
    }
    error_report(
        "tracked-state-invalid",
        "tracked JSONL failed the decision workflow validator",
        issues,
    )
}

/// Convert a frozen adapter/configuration failure into the v1 error report.
///
/// This is public so the CLI can report a missing or incompatible `br` without
/// inventing a second error projection before a `CheckedPlanner` exists.
pub fn report_for_adapter_error(error: BrAdapterError) -> CheckedPlan {
    match error {
        BrAdapterError::MissingExecutable { .. } => error_report(
            "required-command-missing",
            "required command is missing: br",
            vec![issue_with(
                "<command>",
                "required-command-missing",
                [("command", Value::String("br".to_owned()))],
            )],
        ),
        BrAdapterError::InvalidRepositoryRoot { path, reason } => error_report(
            "invalid-repository-root",
            "repository root is invalid",
            vec![issue_with(
                "<repository>",
                "invalid-repository-root",
                [
                    ("path", Value::String(path.display().to_string())),
                    ("detail", Value::String(bounded_text(&reason))),
                ],
            )],
        ),
        BrAdapterError::IncompatibleExecutable { executable, reason } => error_report(
            "incompatible-command",
            "br executable is incompatible",
            vec![issue_with(
                "<command>",
                "incompatible-command",
                [
                    ("path", Value::String(executable.display().to_string())),
                    ("detail", Value::String(bounded_text(&reason))),
                ],
            )],
        ),
        other => {
            let source = other.source_kind().unwrap_or(SourceKind::Ready);
            let reason = other.reason_code();
            let mut extra = Map::new();
            if let Some(diagnostics) = other.diagnostics() {
                if !diagnostics.stderr.is_empty() {
                    extra.insert(
                        "stderr".to_owned(),
                        Value::String(diagnostics.stderr.clone()),
                    );
                }
                if !diagnostics.stdout.is_empty() {
                    extra.insert(
                        "stdout".to_owned(),
                        Value::String(diagnostics.stdout.clone()),
                    );
                }
            }
            let message = bounded_text(&other.to_string());
            extra.insert("detail".to_owned(), Value::String(message.clone()));
            let issue_reason = if matches!(
                other,
                BrAdapterError::NonZero { .. } | BrAdapterError::Spawn { .. }
            ) {
                "canonical-source-command-failed"
            } else {
                reason
            };
            error_report(
                if matches!(
                    other,
                    BrAdapterError::NonZero { .. } | BrAdapterError::Spawn { .. }
                ) {
                    "canonical-source-failure"
                } else {
                    reason
                },
                &format!("canonical {source} command failed: {message}"),
                vec![PlanIssue {
                    id: format!("<{source}>"),
                    reason: issue_reason.to_owned(),
                    extra,
                }],
            )
        }
    }
}

fn error_report(code: &str, message: &str, issues: Vec<PlanIssue>) -> CheckedPlan {
    CheckedPlan {
        schema: PLAN_SCHEMA,
        status: PlanStatus::Error,
        plan: None,
        candidates: None,
        excluded: None,
        evidence: None,
        summary: None,
        error: Some(PlanError {
            code: code.to_owned(),
            message: message.to_owned(),
            issues,
        }),
    }
}

fn issue(id: &str, reason: &str) -> PlanIssue {
    PlanIssue {
        id: id.to_owned(),
        reason: reason.to_owned(),
        extra: Map::new(),
    }
}

fn issue_with<K, const N: usize>(id: &str, reason: &str, values: [(K, Value); N]) -> PlanIssue
where
    K: Into<String>,
{
    let mut extra = Map::new();
    for (key, value) in values {
        extra.insert(key.into(), value);
    }
    PlanIssue {
        id: id.to_owned(),
        reason: reason.to_owned(),
        extra,
    }
}

fn sorted_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut values = ids.map(ToOwned::to_owned).collect::<Vec<_>>();
    values.sort();
    values
}

fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for id in ids {
        *counts.entry(id.to_owned()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(id, count)| (count > 1).then_some(id))
        .collect()
}

fn string_values(values: &[String]) -> Vec<Value> {
    values.iter().cloned().map(Value::String).collect()
}

fn labels_equal_unordered(left: &[String], right: &[String]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn bounded_text(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= crate::br_adapter::MAX_DIAGNOSTIC_TEXT {
        return collapsed;
    }
    let mut output = String::new();
    for character in collapsed.chars() {
        if output.len() + character.len_utf8() + '…'.len_utf8()
            > crate::br_adapter::MAX_DIAGNOSTIC_TEXT
        {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}

impl fmt::Display for CheckedPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        serde_json::to_string(self)
            .map_err(|_| fmt::Error)
            .and_then(|json| formatter.write_str(&json))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};

    use super::{
        ALTERNATE_CONTAINER_LABEL, CONTAINER_LABEL, HUMAN_INPUT_LABEL, IssueView,
        OWNER_DECISION_LABEL, PlanStatus, PlannerInputs, ValidationReport, ValidationStatus,
        parse_ready, parse_scheduler, parse_tracked, parse_tracked_issue, plan,
        report_for_validation,
    };

    #[test]
    fn pure_plan_excludes_owner_decisions_and_keeps_normal_order() {
        let tracked = r#"{"id":"decision","status":"decision","labels":["decision-needed","human-input"]}
{"id":"normal","status":"open","labels":[]}
"#;
        let ready = r#"[{"id":"decision","status":"decision","labels":[]},{"id":"normal","status":"open","labels":[]}]"#;
        let scheduler = r#"{"schema":"br.scheduler.v1","recommendations":[{"rank":1,"issue":{"id":"decision","status":"decision","labels":[]}},{"rank":2,"issue":{"id":"normal","status":"open","labels":[]}}]}"#;
        let report = plan(PlannerInputs {
            ready_json: ready,
            scheduler_json: scheduler,
            tracked_jsonl: tracked,
            tracked_path: Path::new("fixture.jsonl"),
        });
        assert_eq!(report.status, PlanStatus::Error);
        assert_eq!(
            report.error.as_ref().map(|error| error.code.as_str()),
            Some("tracked-state-disagreement")
        );
    }

    #[test]
    fn ready_parser_rejects_every_issue_shape_error() {
        let cases = [
            Value::Null,
            json!({}),
            json!({"id": 1, "status": "open", "labels": []}),
            json!({"id": "", "status": "open", "labels": []}),
            json!({"id": "x" , "labels": []}),
            json!({"id": "x", "status": 1, "labels": []}),
            json!({"id": "x", "status": "open"}),
            json!({"id": "x", "status": "open", "labels": {}}),
            json!({"id": "x", "status": "open", "labels": [1]}),
        ];
        for case in cases {
            let source = serde_json::to_string(&vec![case]).expect("serialize ready case");
            let error = parse_ready(&source).expect_err("malformed ready case must fail");
            assert!(!error.is_empty());
        }
        assert_eq!(
            parse_ready("{}").unwrap_err(),
            "top-level value is not an array"
        );
    }

    #[test]
    fn scheduler_parser_rejects_container_and_issue_shape_errors() {
        assert_eq!(
            parse_scheduler("[]").unwrap_err(),
            "top-level value is not an object"
        );
        assert_eq!(
            parse_scheduler("{}").unwrap_err(),
            "recommendations is not an array"
        );
        assert_eq!(
            parse_scheduler(r#"{"recommendations":{}}"#).unwrap_err(),
            "recommendations is not an array"
        );
        assert_eq!(
            parse_scheduler(r#"{"recommendations":[null]}"#).unwrap_err(),
            "recommendation is not an object"
        );
        assert_eq!(
            parse_scheduler(r#"{"recommendations":[{}]}"#).unwrap_err(),
            "recommendation issue is missing"
        );
        assert_eq!(
            parse_scheduler(r#"{"recommendations":[{"issue":null}]}"#).unwrap_err(),
            "issue is not an object"
        );
        let parsed = parse_scheduler(
            r#"{"recommendations":[{"issue":{"id":"x","status":"open","labels":[]}}]}"#,
        )
        .expect("missing rank is represented as null");
        assert_eq!(parsed[0].rank, Value::Null);
    }

    #[test]
    fn tracked_parser_covers_blank_lines_and_field_shapes() {
        let valid = parse_tracked("\n{\"id\":\"x\",\"status\":\"open\"}\n\n")
            .expect("blank lines are ignored by parser");
        assert_eq!(valid.len(), 1);
        let cases = [
            ("null", "tracked issue is not an object"),
            ("{}", "tracked issue id is missing or not a string"),
            (
                r#"{"id":1,"status":"open"}"#,
                "tracked issue id is missing or not a string",
            ),
            (
                r#"{"id":"x"}"#,
                "tracked issue status is missing or not a string",
            ),
            (
                r#"{"id":"x","status":1}"#,
                "tracked issue status is missing or not a string",
            ),
            (
                r#"{"id":"x","status":"open","labels":{}}"#,
                "tracked issue labels are not an array",
            ),
            (
                r#"{"id":"x","status":"open","labels":[1]}"#,
                "tracked issue labels must contain only strings",
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(parse_tracked(source).unwrap_err(), expected);
        }
    }

    #[test]
    fn issue_view_exclusion_reasons_cover_status_labels_and_containers() {
        let make = |id: &str, status: &str, labels: &[&str], issue_type: Option<&str>| {
            let mut raw = serde_json::Map::new();
            raw.insert("id".to_owned(), Value::String(id.to_owned()));
            raw.insert("status".to_owned(), Value::String(status.to_owned()));
            raw.insert(
                "labels".to_owned(),
                Value::Array(
                    labels
                        .iter()
                        .map(|label| Value::String((*label).to_owned()))
                        .collect(),
                ),
            );
            IssueView {
                raw,
                id: id.to_owned(),
                status: status.to_owned(),
                labels: labels.iter().map(|label| (*label).to_owned()).collect(),
                issue_type: issue_type.map(ToOwned::to_owned),
            }
        };
        assert_eq!(
            make("decision", "decision", &[], None).exclusion(),
            Some(("owner-decision".to_owned(), "owner-input".to_owned()))
        );
        assert_eq!(
            make("closed", "closed", &[OWNER_DECISION_LABEL], None).exclusion(),
            Some((
                "closed-decision-history".to_owned(),
                "historical-owner-decision".to_owned()
            ))
        );
        assert_eq!(
            make("blocked", "blocked", &[], None).exclusion(),
            Some(("blocked".to_owned(), "dependency-or-policy".to_owned()))
        );
        assert_eq!(
            make("deferred", "deferred", &[], None).exclusion(),
            Some(("deferred".to_owned(), "deferred-by-workflow".to_owned()))
        );
        assert_eq!(
            make("container", "open", &[ALTERNATE_CONTAINER_LABEL], None).exclusion(),
            Some(("container".to_owned(), "container-not-leaf-work".to_owned()))
        );
        assert_eq!(
            make("epic", "open", &[], Some("epic")).exclusion(),
            Some(("container".to_owned(), "container-not-leaf-work".to_owned()))
        );
        assert_eq!(
            make("other", "in_progress", &[], None).exclusion(),
            Some((
                "status-not-ready".to_owned(),
                "status:in_progress".to_owned()
            ))
        );
        assert_eq!(
            make("drift", "open", &[OWNER_DECISION_LABEL], None).exclusion(),
            Some((
                "owner-decision".to_owned(),
                "decision-label-drift".to_owned()
            ))
        );
        assert_eq!(make("ready", "open", &[], None).exclusion(), None);
        assert!(make("marked", "open", &[OWNER_DECISION_LABEL], None).is_decision_marked());
        assert!(make("marked", "open", &[HUMAN_INPUT_LABEL], None).is_decision_marked());
        assert!(make("container", "open", &[CONTAINER_LABEL], None).is_container());
    }

    #[test]
    fn tracked_issue_parser_accepts_optional_labels_and_rejects_each_shape() {
        let optional = parse_tracked_issue(&json!({"id":"x","status":"open"}))
            .expect("labels are optional in tracked records");
        assert!(optional.labels.is_empty());
        let cases = [
            (Value::Null, "tracked issue is not an object"),
            (json!({}), "tracked issue id is missing or not a string"),
            (
                json!({"id":"x"}),
                "tracked issue status is missing or not a string",
            ),
            (
                json!({"id":"x","status":"open","labels":null}),
                "tracked issue labels are not an array",
            ),
            (
                json!({"id":"x","status":"open","labels":[null]}),
                "tracked issue labels must contain only strings",
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(parse_tracked_issue(&value).unwrap_err(), expected);
        }
    }

    #[test]
    fn empty_validation_findings_get_a_stable_fallback_issue() {
        let report = report_for_validation(ValidationReport {
            schema: "test",
            status: ValidationStatus::Invalid,
            path: "fixture".to_owned(),
            findings: Vec::new(),
            truncated: false,
        });
        assert_eq!(report.status, PlanStatus::Error);
        let error = report.error.expect("error report");
        assert_eq!(error.code, "tracked-state-invalid");
        assert_eq!(error.issues[0].id, "<tracked>");
        assert_eq!(error.issues[0].reason, "tracked-decision-state");
    }
}
