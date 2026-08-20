//! Immutable synchronization-plan and explanation types.
//!
//! Every selected or rejected catalog item carries a stable reason;
//! operations retain source precedence and destination order; plan
//! serialization is deterministic.  The domain type is pure data: no
//! filesystem mutation method exists on it.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The plan wire schema.
pub const PLAN_SCHEMA: &str = "omnirepo.sync-plan.v1";

/// Why an item was selected or rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanDecision {
    /// The item is included because it is the declared winner for its
    /// target.
    Selected { reason: String },
    /// The item lost a collision or its source is unavailable.
    Rejected { reason: String },
}

/// One planned item with its explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanItem {
    pub id: String,
    pub target: String,
    /// The owning source in declared precedence order.
    pub source: String,
    /// The source-relative file path that carries the payload.
    pub source_path: String,
    pub source_order: usize,
    /// The item kind (whole file or section).
    pub kind: crate::source::ItemKind,
    /// The named section for section items; None for whole files.
    pub section: Option<crate::configuration::SectionId>,
    pub decision: PlanDecision,
}

/// The immutable per-repository synchronization plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncPlan {
    pub schema: String,
    pub destination: String,
    /// Items in source precedence then declared order.
    pub items: Vec<PlanItem>,
}

impl SyncPlan {
    /// Build the plan from resolved items; the caller supplies the
    /// per-item decision already computed by the resolution truth table.
    pub fn new(destination: impl Into<String>, items: Vec<PlanItem>) -> Self {
        Self {
            schema: PLAN_SCHEMA.to_owned(),
            destination: destination.into(),
            items,
        }
    }

    /// Deterministic serialization: schema, destination, then every item
    /// in plan order (source precedence, then declared order).
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{PLAN_SCHEMA} destination={}\n", self.destination));
        for item in &self.items {
            let decision = match &item.decision {
                PlanDecision::Selected { .. } => "selected",
                PlanDecision::Rejected { .. } => "rejected",
            };
            let section = match &item.section {
                Some(section) => format!(" section={section}"),
                None => String::new(),
            };
            out.push_str(&format!(
                "item={} target={} source={} order={}{section} {decision}\n",
                item.id, item.target, item.source, item.source_order
            ));
        }
        out
    }
}

/// Plan construction failures.
#[derive(Debug)]
pub enum PlanError {
    Empty { destination: String },
    DuplicateItem { id: String },
    DuplicateTarget { target: String },
    DuplicateSection { target: String, section: String },
    IncompatibleClaims { target: String },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { destination } => {
                write!(formatter, "plan for {destination} has no items")
            }
            Self::DuplicateItem { id } => write!(formatter, "plan item {id} is duplicated"),
            Self::DuplicateTarget { target } => write!(
                formatter,
                "plan target {target} carries more than one whole-file claim"
            ),
            Self::DuplicateSection { target, section } => write!(
                formatter,
                "plan target {target} carries section {section} more than once"
            ),
            Self::IncompatibleClaims { target } => write!(
                formatter,
                "plan target {target} carries both a whole-file claim and a section claim"
            ),
        }
    }
}
impl Error for PlanError {}

/// Validate a plan before it is committed: non-empty, unique selected
/// items, one authority per whole file and per named section,
/// deterministic order already enforced by construction.
pub fn validate_plan(plan: &SyncPlan) -> Result<(), PlanError> {
    let selected = plan
        .items
        .iter()
        .filter(|item| matches!(item.decision, PlanDecision::Selected { .. }))
        .collect::<Vec<_>>();
    // Rejected losers never execute: a plan with no selected item is
    // empty work, however many documented losers it carries.
    if selected.is_empty() {
        return Err(PlanError::Empty {
            destination: plan.destination.clone(),
        });
    }
    let mut seen = Vec::with_capacity(selected.len());
    for item in &selected {
        if seen.contains(&item.id) {
            return Err(PlanError::DuplicateItem {
                id: item.id.clone(),
            });
        }
        seen.push(item.id.clone());
    }
    for (index, item) in selected.iter().enumerate() {
        for other in &selected[..index] {
            if item.target != other.target {
                continue;
            }
            match (&item.section, &other.section) {
                (Some(section), Some(other_section)) if section == other_section => {
                    return Err(PlanError::DuplicateSection {
                        target: item.target.clone(),
                        section: section.as_str().to_owned(),
                    });
                }
                (Some(_), Some(_)) => {}
                (None, None) => {
                    return Err(PlanError::DuplicateTarget {
                        target: item.target.clone(),
                    });
                }
                _ => {
                    return Err(PlanError::IncompatibleClaims {
                        target: item.target.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod sync_plan_tests;
