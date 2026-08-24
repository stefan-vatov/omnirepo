//! The `doctor` command: one deep machine diagnostic without effects.
//!
//! Doctor runs the same effect-free planning prefix as `sync` — machine
//! configuration, source catalog, pinned declarations, per-repository
//! policies, bindings, and the per-repository plans — then reports what
//! it finds instead of applying anything: source availability,
//! cross-source conflicts and their declared winners, invalid
//! declarations, and destination formats that carry no delimiter syntax.
//! Doctor reads only each destination's `.omnirepo.yaml` repository
//! policy, never managed content; it writes nothing, creates no run
//! record, and is never a fleet run.

#![allow(dead_code)]

#[cfg(test)]
mod doctor_tests;

use crate::configuration::{Discovery, discover};
use crate::lifecycle::fleet_dispatch::{FleetPlanning, plan_configured_fleet};
use crate::lifecycle::sync_plan::{PlanDecision, SyncPlan};
use crate::source::{CatalogState, ItemKind};
use std::path::Path;

/// One diagnostic line with its severity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Finding {
    /// Informational: the state is deliberate and healthy.
    Info(String),
    /// A problem that would fail or skip work in a real `sync` run.
    Problem(String),
}

/// The complete doctor report: deterministic ordered findings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DoctorReport {
    pub findings: Vec<Finding>,
}

impl DoctorReport {
    pub fn healthy(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| matches!(finding, Finding::Problem(_)))
    }

    fn info(&mut self, line: impl Into<String>) {
        // Findings interpolate raw configuration and error text; the
        // shared redaction rule keeps credential-bearing fragments out
        // of the report, same as run-record evidence.
        self.findings.push(Finding::Info(
            crate::lifecycle::diagnostic_aggregation::redact(&line.into()),
        ));
    }

    fn problem(&mut self, line: impl Into<String>) {
        self.findings.push(Finding::Problem(
            crate::lifecycle::diagnostic_aggregation::redact(&line.into()),
        ));
    }

    /// Render the report as stable human-readable lines.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for finding in &self.findings {
            match finding {
                Finding::Info(line) => {
                    out.push_str("ok      ");
                    out.push_str(line);
                }
                Finding::Problem(line) => {
                    out.push_str("problem ");
                    out.push_str(line);
                }
            }
            out.push('\n');
        }
        out
    }
}

/// Build the doctor report for one machine home.
pub fn diagnose(home: &Path) -> DoctorReport {
    let mut report = DoctorReport::default();
    let discovery = match discover(home) {
        Ok(discovery) => discovery,
        Err(error) => {
            report.problem(format!("machine configuration: {error}"));
            return report;
        }
    };
    let config = match discovery {
        Discovery::Absent => {
            report.info(
                "machine configuration: absent (<HOME>/.omnirepo/config.yaml); the fleet is empty",
            );
            return report;
        }
        Discovery::Present(config) => {
            report.info("machine configuration: valid");
            config
        }
    };
    let FleetPlanning { catalog, plans, .. } = match plan_configured_fleet(&config) {
        Ok(planning) => planning,
        Err(error) => {
            report.problem(format!("planning: {error}"));
            return report;
        }
    };
    for state in catalog.entries() {
        match state {
            CatalogState::Complete { source, revision } => {
                report.info(format!(
                    "source {}: complete at {}",
                    source.as_str(),
                    revision.as_str()
                ));
            }
            CatalogState::Shadowed { source, by } => {
                report.info(format!(
                    "source {}: shadowed by the higher-precedence source {}",
                    source.as_str(),
                    by.as_str()
                ));
            }
            CatalogState::Unavailable { source, reason } => {
                report.problem(format!("source {}: unavailable: {reason}", source.as_str()));
            }
        }
    }
    for plan in &plans {
        match &plan.plan {
            Ok(sync_plan) => diagnose_plan(&mut report, &plan.repository, sync_plan),
            Err(reason) => {
                report.problem(format!("repository {}: {reason}", plan.repository));
            }
        }
    }
    report
}

/// Diagnose one repository's resolved plan.
fn diagnose_plan(report: &mut DoctorReport, repository: &str, plan: &SyncPlan) {
    for item in &plan.items {
        match &item.decision {
            PlanDecision::Selected { .. } => {
                let shape = match (&item.kind, &item.section) {
                    (ItemKind::Section, Some(section)) => {
                        format!("section {section} of {}", item.target)
                    }
                    _ => format!("whole file {}", item.target),
                };
                report.info(format!(
                    "repository {repository}: item {} from source {} manages {shape}",
                    item.id, item.source
                ));
                // A selected section whose destination format carries no
                // delimiter syntax would fail at sync time: surface it now.
                if item.kind == ItemKind::Section
                    && let Err(error) = crate::managed_content::lookup_by_extension(&item.target)
                {
                    report.problem(format!(
                        "repository {repository}: item {} targets {}: {error}",
                        item.id, item.target
                    ));
                }
            }
            PlanDecision::Rejected { reason } => {
                report.info(format!(
                    "repository {repository}: item {} from source {} is shadowed: {reason}",
                    item.id, item.source
                ));
            }
        }
    }
}
