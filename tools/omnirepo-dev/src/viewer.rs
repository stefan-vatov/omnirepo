//! Decision-aware projections for the repository's static Beads Viewer export.
//!
//! The Viewer is display tooling. It does not choose work, close owner
//! decisions, or treat raw `bv` recommendations as authority. This module
//! consumes the versioned export contract and produces deterministic rows for
//! the graph, list, detail, filter, and triage views.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// The versioned input schema owned by the Viewer fixture contract.
pub const EXPORT_SCHEMA: &str = "omnirepo.viewer-export-fixture.v1";

/// The checked plan schema that is allowed to contribute actionable IDs.
pub const CHECKED_PLAN_SCHEMA: &str = "omnirepo.checked-agent-plan.v1";

const CANONICAL_ACTIONABLE_SOURCES: [&str; 2] = ["br-ready", "checked-agent-plan"];
const REQUIRED_CATEGORIES: [&str; 13] = [
    "actionable",
    "active",
    "blocked",
    "closed",
    "closed-decision",
    "container",
    "deferred",
    "invalid",
    "invalid-owner-state",
    "open-non-actionable",
    "owner-decision",
    "retired",
    "stale-export",
];

/// A parseable versioned Viewer export.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ViewerExport {
    /// The versioned fixture/export schema identifier.
    pub schema: String,
    /// Independent export cases. Each case becomes one deterministic view.
    pub cases: Vec<ViewerCase>,
    /// The status and wording contract used by every case.
    pub contract: ViewerContract,
    /// Tracker statuses that the export promises to classify.
    pub required_tracker_statuses: Vec<String>,
}

/// One independent set of tracker rows and canonical planning evidence.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ViewerCase {
    /// Stable case name used in diagnostics and output.
    pub name: String,
    /// Evidence describing whether the case is canonical, invalid, or stale.
    pub evidence: Evidence,
    /// Tracker rows to project.
    pub rows: Vec<TrackerRow>,
    /// Canonical and advisory planning inputs.
    pub sources: Sources,
}

/// Evidence classification for one export case.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Evidence {
    /// `canonical`, `invalid-tracker`, or `stale-export` in the contract.
    pub kind: String,
}

/// A tracker issue row consumed by the adapter.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrackerRow {
    /// Globally unique issue identifier within the case.
    pub id: String,
    /// Beads issue type. The Viewer preserves it without assigning semantics.
    pub issue_type: String,
    /// Labels used for owner-decision and workstream classification.
    pub labels: Vec<String>,
    /// Beads lifecycle status.
    pub status: String,
    /// Creation provenance for closed decision detection.
    pub created_at: Option<String>,
    /// Creator provenance for closed decision detection.
    pub created_by: Option<String>,
    /// Close provenance for closed decision detection.
    pub closed_at: Option<String>,
    /// Close reason provenance for closed decision detection.
    pub close_reason: Option<String>,
}

/// The two canonical action sources and the quarantined raw Viewer source.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sources {
    /// IDs reported ready by `br`.
    pub br_ready_ids: Vec<String>,
    /// Output from the checked autonomous plan wrapper.
    pub checked_plan: CheckedPlan,
    /// Raw `bv` recommendations. These are retained only as evidence.
    pub raw_bv_recommended_ids: Vec<String>,
}

/// The checked autonomous plan evidence.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CheckedPlan {
    /// IDs approved by the checked wrapper.
    pub actionable_ids: Vec<String>,
    /// Evidence that raw `bv` was advisory only.
    pub evidence: RawViewerEvidence,
    /// Checked plan schema identifier.
    pub schema: String,
    /// `ok` for canonical input; another value fails closed.
    pub status: String,
}

/// Evidence about the raw Viewer recommendation source.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RawViewerEvidence {
    /// The only accepted value is `advisory-only`.
    pub raw_bv: String,
}

/// The decision-aware status, wording, and source trust contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ViewerContract {
    /// Ordered source names. Order is part of the declared contract.
    pub canonical_actionable_sources: Vec<String>,
    /// Required provenance fields for a closed owner decision.
    pub closed_decision_provenance: Vec<String>,
    /// Owner-only decision queue rules.
    pub owner_queue: OwnerQueue,
    /// Trust label for raw `bv` output.
    pub raw_bv: String,
    /// Mapping from tracker status to display category.
    pub status_classes: BTreeMap<String, String>,
    /// Stable user-visible wording for each display category.
    pub wording: BTreeMap<String, String>,
}

/// Owner-only queue rules from the export contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OwnerQueue {
    /// The only allowed action for this queue.
    pub action: String,
    /// Labels that identify decision provenance.
    pub required_labels: Vec<String>,
    /// Status identifying active owner decisions.
    pub status: String,
}

/// A deterministic, decision-safe projection for one static Viewer case.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaseProjection {
    /// Stable source case name.
    pub name: String,
    /// Unique graph nodes sorted by issue ID.
    pub graph: Vec<GraphNode>,
    /// List rows sorted by issue ID.
    pub list: Vec<ProjectionRow>,
    /// Detail cards keyed by issue ID.
    pub details: BTreeMap<String, DetailProjection>,
    /// Category filters, including empty categories.
    pub filters: BTreeMap<String, Vec<String>>,
    /// Canonical triage projection.
    pub triage: TriageProjection,
    /// Counts for every declared category, including zeroes.
    pub counts: BTreeMap<String, usize>,
    /// Active owner-decision IDs. These are never actionable.
    pub owner_queue_ids: Vec<String>,
    /// Rows rejected as invalid owner/tracker state.
    pub invalid_ids: Vec<String>,
    /// Rows rejected because the export is stale.
    pub stale_ids: Vec<String>,
}

impl CaseProjection {
    /// Return all rows in a category in deterministic order.
    pub fn filter(&self, category: &str) -> Vec<&ProjectionRow> {
        self.list
            .iter()
            .filter(|row| row.category == category)
            .collect()
    }

    /// Return the detail card for an issue ID, if the export contains it.
    pub fn detail(&self, id: &str) -> Option<&DetailProjection> {
        self.details.get(id)
    }
}

/// The complete adaptation result for all cases in one export.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ViewerProjection {
    /// The input schema identifier retained for traceability.
    pub schema: String,
    /// Deterministic case projections in source order.
    pub cases: Vec<CaseProjection>,
}

/// A unique node in the graph projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphNode {
    /// Stable issue identifier.
    pub id: String,
    /// Display category used by graph badges and filters.
    pub category: String,
    /// Whether the node is eligible for the canonical triage view.
    pub actionable: bool,
}

/// A deterministic list row shared by list and detail projections.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionRow {
    /// Stable issue identifier.
    pub id: String,
    /// Display category.
    pub category: String,
    /// Whether canonical evidence permits agent work.
    pub actionable: bool,
    /// Stable user-facing wording.
    pub wording: String,
}

/// A detail card with an explicit badge and safe wording.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DetailProjection {
    /// Stable issue identifier.
    pub id: String,
    /// Badge/category shown by the Viewer.
    pub badge: String,
    /// Owner-safe wording shown by the Viewer.
    pub wording: String,
    /// Whether this card is in the canonical actionable queue.
    pub actionable: bool,
}

/// The only IDs that may appear in canonical Viewer triage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TriageProjection {
    /// `br-ready ∩ checked-agent-plan`, filtered to safe actionable rows.
    pub actionable_ids: Vec<String>,
    /// Raw `bv` remains evidence, never an action source.
    pub raw_bv: String,
}

/// Errors raised while validating or projecting an export.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewerAdapterError {
    /// The top-level schema is not the contract version this adapter owns.
    SchemaMismatch { expected: String, actual: String },
    /// The declared contract is internally inconsistent.
    InvalidContract { reason: String },
    /// A case contains the same issue ID more than once.
    DuplicateIssueId { case: String, id: String },
    /// Canonical action sources disagree for a case.
    ActionableSourceMismatch { case: String },
    /// A canonical action source has no uniquely identified tracker row.
    SourceRowMismatch { case: String, id: String },
    /// A checked plan has the wrong schema or trust evidence.
    InvalidCheckedPlan { case: String, reason: String },
}

impl fmt::Display for ViewerAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaMismatch { expected, actual } => write!(
                formatter,
                "unsupported viewer export schema: expected {expected}, got {actual}"
            ),
            Self::InvalidContract { reason } => {
                write!(formatter, "invalid viewer contract: {reason}")
            }
            Self::DuplicateIssueId { case, id } => {
                write!(formatter, "duplicate issue id {id} in viewer case {case}")
            }
            Self::ActionableSourceMismatch { case } => {
                write!(
                    formatter,
                    "canonical action sources disagree in viewer case {case}"
                )
            }
            Self::SourceRowMismatch { case, id } => write!(
                formatter,
                "canonical source id {id} has no tracker row in viewer case {case}"
            ),
            Self::InvalidCheckedPlan { case, reason } => {
                write!(
                    formatter,
                    "invalid checked plan in viewer case {case}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for ViewerAdapterError {}

/// Adapt a parsed static export into deterministic Viewer projections.
///
/// The caller owns parsing so the module can be used by both the private CLI
/// and fixture-backed integration tests without duplicating projection logic.
/// Raw `bv` recommendations never enter `triage.actionable_ids`. Owner
/// decisions, invalid states, and stale exports are always non-actionable.
///
/// # Errors
///
/// Returns [`ViewerAdapterError`] when the versioned contract is malformed,
/// canonical sources disagree, or an issue ID is duplicated.
pub fn adapt_export(export: ViewerExport) -> Result<ViewerProjection, ViewerAdapterError> {
    validate_contract(&export)?;

    let mut cases = Vec::with_capacity(export.cases.len());
    for case in export.cases {
        cases.push(project_case(&export.contract, case)?);
    }

    Ok(ViewerProjection {
        schema: export.schema,
        cases,
    })
}

fn validate_contract(export: &ViewerExport) -> Result<(), ViewerAdapterError> {
    if export.schema != EXPORT_SCHEMA {
        return Err(ViewerAdapterError::SchemaMismatch {
            expected: EXPORT_SCHEMA.to_owned(),
            actual: export.schema.clone(),
        });
    }

    if export.contract.canonical_actionable_sources
        != CANONICAL_ACTIONABLE_SOURCES
            .iter()
            .map(|source| (*source).to_owned())
            .collect::<Vec<_>>()
    {
        return Err(ViewerAdapterError::InvalidContract {
            reason: "canonical actionable sources must be br-ready then checked-agent-plan"
                .to_owned(),
        });
    }
    if export.contract.raw_bv != "advisory-only" {
        return Err(ViewerAdapterError::InvalidContract {
            reason: "raw bv must be advisory-only".to_owned(),
        });
    }
    if export.contract.owner_queue.action != "owner-only"
        || export.contract.owner_queue.status != "decision"
        || export.contract.owner_queue.required_labels.is_empty()
    {
        return Err(ViewerAdapterError::InvalidContract {
            reason: "owner queue must be owner-only, decision status, and label-gated".to_owned(),
        });
    }
    if export.contract.closed_decision_provenance
        != ["created_at", "created_by", "closed_at", "close_reason"]
            .iter()
            .map(|field| (*field).to_owned())
            .collect::<Vec<_>>()
    {
        return Err(ViewerAdapterError::InvalidContract {
            reason: "closed decision provenance fields differ".to_owned(),
        });
    }

    let required_statuses = export
        .required_tracker_statuses
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let declared_statuses = export
        .contract
        .status_classes
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if required_statuses != declared_statuses {
        return Err(ViewerAdapterError::InvalidContract {
            reason: "required tracker statuses and status classes differ".to_owned(),
        });
    }

    for category in REQUIRED_CATEGORIES {
        if !export.contract.wording.contains_key(category) {
            return Err(ViewerAdapterError::InvalidContract {
                reason: format!("missing wording for category {category}"),
            });
        }
    }
    if export
        .contract
        .status_classes
        .values()
        .any(|category| !export.contract.wording.contains_key(category))
    {
        return Err(ViewerAdapterError::InvalidContract {
            reason: "status class has no declared wording".to_owned(),
        });
    }

    Ok(())
}

fn project_case(
    contract: &ViewerContract,
    case: ViewerCase,
) -> Result<CaseProjection, ViewerAdapterError> {
    let candidate_actionable = canonical_actionable_ids(&case)?;
    let mut seen = BTreeSet::new();
    for row in &case.rows {
        if !seen.insert(row.id.clone()) {
            return Err(ViewerAdapterError::DuplicateIssueId {
                case: case.name.clone(),
                id: row.id.clone(),
            });
        }
    }
    for id in &candidate_actionable {
        if !seen.contains(id) {
            return Err(ViewerAdapterError::SourceRowMismatch {
                case: case.name.clone(),
                id: id.clone(),
            });
        }
    }
    for row in &case.rows {
        if candidate_actionable.contains(&row.id)
            && projected_category(&case, row, contract) != "actionable"
        {
            return Err(ViewerAdapterError::ActionableSourceMismatch {
                case: case.name.clone(),
            });
        }
    }

    let mut list = Vec::with_capacity(case.rows.len());
    let mut graph = Vec::with_capacity(case.rows.len());
    let mut details = BTreeMap::new();
    let mut counts = contract
        .wording
        .keys()
        .map(|category| (category.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut filters = contract
        .wording
        .keys()
        .map(|category| (category.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut owner_queue_ids = Vec::new();
    let mut invalid_ids = Vec::new();
    let mut stale_ids = Vec::new();

    for row in &case.rows {
        let mut category = projected_category(&case, row, contract);
        if category == "actionable" && !candidate_actionable.contains(&row.id) {
            category = "open-non-actionable".to_owned();
        }
        let safe_actionable = category == "actionable";
        let wording = contract
            .wording
            .get(&category)
            .cloned()
            .unwrap_or_else(|| "Invalid tracker state".to_owned());

        list.push(ProjectionRow {
            id: row.id.clone(),
            category: category.clone(),
            actionable: safe_actionable,
            wording: wording.clone(),
        });
        graph.push(GraphNode {
            id: row.id.clone(),
            category: category.clone(),
            actionable: safe_actionable,
        });
        details.insert(
            row.id.clone(),
            DetailProjection {
                id: row.id.clone(),
                badge: category.clone(),
                wording,
                actionable: safe_actionable,
            },
        );
        *counts.entry(category.clone()).or_default() += 1;
        filters
            .entry(category.clone())
            .or_default()
            .push(row.id.clone());

        if is_owner_decision(row, contract) {
            owner_queue_ids.push(row.id.clone());
        }
        if matches!(category.as_str(), "invalid" | "invalid-owner-state") {
            invalid_ids.push(row.id.clone());
        }
        if category == "stale-export" {
            stale_ids.push(row.id.clone());
        }
    }

    list.sort_by(|left, right| left.id.cmp(&right.id));
    graph.sort_by(|left, right| left.id.cmp(&right.id));
    owner_queue_ids.sort();
    invalid_ids.sort();
    stale_ids.sort();
    for ids in filters.values_mut() {
        ids.sort();
    }

    let actionable_ids = list
        .iter()
        .filter(|row| row.actionable)
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();

    Ok(CaseProjection {
        name: case.name,
        graph,
        list,
        details,
        filters,
        triage: TriageProjection {
            actionable_ids,
            raw_bv: "advisory-only".to_owned(),
        },
        counts,
        owner_queue_ids,
        invalid_ids,
        stale_ids,
    })
}

fn canonical_actionable_ids(case: &ViewerCase) -> Result<BTreeSet<String>, ViewerAdapterError> {
    if case.evidence.kind != "canonical" {
        return Ok(BTreeSet::new());
    }

    if case.sources.checked_plan.schema != CHECKED_PLAN_SCHEMA {
        return Err(ViewerAdapterError::InvalidCheckedPlan {
            case: case.name.clone(),
            reason: format!(
                "expected {CHECKED_PLAN_SCHEMA}, got {}",
                case.sources.checked_plan.schema
            ),
        });
    }
    if case.sources.checked_plan.evidence.raw_bv != "advisory-only" {
        return Err(ViewerAdapterError::InvalidCheckedPlan {
            case: case.name.clone(),
            reason: "raw bv evidence must be advisory-only".to_owned(),
        });
    }
    if case.sources.checked_plan.status != "ok" {
        return Err(ViewerAdapterError::InvalidCheckedPlan {
            case: case.name.clone(),
            reason: "canonical cases require an ok checked plan".to_owned(),
        });
    }

    let br_ready = case
        .sources
        .br_ready_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let checked_plan = case
        .sources
        .checked_plan
        .actionable_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if br_ready.len() != case.sources.br_ready_ids.len()
        || checked_plan.len() != case.sources.checked_plan.actionable_ids.len()
        || br_ready != checked_plan
    {
        return Err(ViewerAdapterError::ActionableSourceMismatch {
            case: case.name.clone(),
        });
    }
    Ok(br_ready)
}

fn projected_category(case: &ViewerCase, row: &TrackerRow, contract: &ViewerContract) -> String {
    if case.evidence.kind == "stale-export" {
        return "stale-export".to_owned();
    }
    if row.status == "open"
        && contract
            .owner_queue
            .required_labels
            .iter()
            .all(|label| row.labels.iter().any(|candidate| candidate == label))
    {
        return "invalid-owner-state".to_owned();
    }
    if has_closed_decision_provenance(row, contract) {
        return "closed-decision".to_owned();
    }
    if row.status == "open"
        && row
            .labels
            .iter()
            .any(|label| label == "workstream-container")
    {
        return "container".to_owned();
    }

    contract
        .status_classes
        .get(&row.status)
        .cloned()
        .unwrap_or_else(|| "invalid".to_owned())
}

fn is_owner_decision(row: &TrackerRow, contract: &ViewerContract) -> bool {
    row.status == contract.owner_queue.status
        && contract
            .status_classes
            .get(&row.status)
            .is_some_and(|category| category == "owner-decision")
}

fn has_closed_decision_provenance(row: &TrackerRow, contract: &ViewerContract) -> bool {
    row.status == "closed"
        && contract
            .owner_queue
            .required_labels
            .iter()
            .all(|label| row.labels.iter().any(|candidate| candidate == label))
        && row.created_at.is_some()
        && row.created_by.is_some()
        && row.closed_at.is_some()
        && row.close_reason.is_some()
}
