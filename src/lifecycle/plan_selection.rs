//! Absent-policy inference and explicit-policy selection.
//!
//! The decision table: an explicit policy selects every item whose id is
//! included and not excluded; an absent policy infers the canonical
//! default (every declared item) with a stable explanation.  Unknown or
//! invalid selectors fail typed rather than infer; include/exclude
//! explanations are stable.

#![allow(dead_code)]

use super::sync_plan::{PlanDecision, PlanItem};
use std::{error::Error, fmt};

/// The policy applied to one plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Policy {
    Explicit {
        include: Vec<String>,
        exclude: Vec<String>,
    },
    Absent,
}

/// One selection decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionDecision {
    Selected { reason: String },
    Rejected { reason: String },
}

/// Selection failures: unknown or invalid selectors never infer.
#[derive(Debug)]
pub enum SelectionError {
    UnknownSelector { selector: String },
    ConflictingSelector { id: String },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSelector { selector } => {
                write!(formatter, "selector {selector:?} matches no declared item")
            }
            Self::ConflictingSelector { id } => {
                write!(formatter, "item {id} is both included and excluded")
            }
        }
    }
}
impl Error for SelectionError {}

/// One selection outcome with its stable explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    pub item: PlanItem,
    pub decision: SelectionDecision,
}

/// Apply the decision table to every item of the plan.
///
/// Explicit: included and not excluded → Selected; included and excluded →
/// typed conflict; excluded → Rejected; not mentioned → Rejected (explicit
/// scope).  Absent: every declared winner → Selected (canonical default);
/// every declared loser stays Rejected with its plan reason.
pub fn select_items(items: &[PlanItem], policy: &Policy) -> Result<Vec<Selection>, SelectionError> {
    match policy {
        Policy::Absent => Ok(items
            .iter()
            .map(|item| Selection {
                item: item.clone(),
                decision: match &item.decision {
                    PlanDecision::Selected { reason } => SelectionDecision::Selected {
                        reason: format!("absent policy: {reason}"),
                    },
                    PlanDecision::Rejected { reason } => SelectionDecision::Rejected {
                        reason: format!("absent policy: {reason}"),
                    },
                },
            })
            .collect()),
        Policy::Explicit { include, exclude } => {
            // Unknown selectors fail rather than infer.
            for selector in include.iter().chain(exclude.iter()) {
                if !items.iter().any(|item| &item.id == selector) {
                    return Err(SelectionError::UnknownSelector {
                        selector: selector.clone(),
                    });
                }
            }
            let mut selections = Vec::with_capacity(items.len());
            for item in items {
                let included = include.iter().any(|id| id == &item.id);
                let excluded = exclude.iter().any(|id| id == &item.id);
                if included && excluded {
                    return Err(SelectionError::ConflictingSelector {
                        id: item.id.clone(),
                    });
                }
                let decision = if included {
                    SelectionDecision::Selected {
                        reason: "explicit include".to_owned(),
                    }
                } else if excluded {
                    SelectionDecision::Rejected {
                        reason: "explicit exclude".to_owned(),
                    }
                } else {
                    SelectionDecision::Rejected {
                        reason: "outside the explicit scope".to_owned(),
                    }
                };
                selections.push(Selection {
                    item: item.clone(),
                    decision,
                });
            }
            Ok(selections)
        }
    }
}

#[cfg(test)]
mod plan_selection_tests;
