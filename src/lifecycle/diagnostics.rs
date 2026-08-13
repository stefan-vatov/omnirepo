//! Stable preflight diagnostic codes with authority-tier context.
//!
//! Every preflight failure maps to exactly one stable code, stage,
//! authority path and identity, optional field location, affected scope,
//! and safe remediation.  Display formatting is a separate concern: the
//! diagnostic is data; rendering is a pure function.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// The preflight stage that produced the diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticStage {
    Configuration,
    Source,
    Plan,
    Admission,
    Verification,
    Delivery,
}

impl DiagnosticStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Source => "source",
            Self::Plan => "plan",
            Self::Admission => "admission",
            Self::Verification => "verification",
            Self::Delivery => "delivery",
        }
    }
}

/// The affected scope of a diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AffectedScope {
    /// The whole run is affected.
    Global,
    /// One repository is affected.
    Repository { repository: String },
    /// One item is affected.
    Item { repository: String, item: String },
}

/// One stable diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// The stable code, e.g. "config-malformed" or "source-unavailable".
    pub code: &'static str,
    pub stage: DiagnosticStage,
    /// The owning authority path and identity context.
    pub authority_path: String,
    pub authority_identity: String,
    /// The optional field location within the authority input.
    pub field: Option<String>,
    pub scope: AffectedScope,
    /// Safe remediation text (never includes secrets or raw output).
    pub remediation: String,
}

/// Diagnostic construction failures.
#[derive(Debug)]
pub enum DiagnosticError {
    EmptyCode,
    EmptyRemediation,
}

impl fmt::Display for DiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCode => write!(formatter, "a diagnostic needs a non-empty code"),
            Self::EmptyRemediation => {
                write!(formatter, "a diagnostic needs a non-empty remediation")
            }
        }
    }
}
impl Error for DiagnosticError {}

/// Build one diagnostic; empty codes and remediations fail typed.
pub fn diagnostic(
    code: &'static str,
    stage: DiagnosticStage,
    authority_path: impl Into<String>,
    authority_identity: impl Into<String>,
    field: Option<String>,
    scope: AffectedScope,
    remediation: impl Into<String>,
) -> Result<Diagnostic, DiagnosticError> {
    if code.is_empty() {
        return Err(DiagnosticError::EmptyCode);
    }
    let remediation = remediation.into();
    if remediation.is_empty() {
        return Err(DiagnosticError::EmptyRemediation);
    }
    Ok(Diagnostic {
        code,
        stage,
        authority_path: authority_path.into(),
        authority_identity: authority_identity.into(),
        field,
        scope,
        remediation,
    })
}

/// Display formatting is separate from the diagnostic data: one stable
/// one-line render, no ANSI, no secrets.
pub fn render_diagnostic(diagnostic: &Diagnostic) -> String {
    let scope = match &diagnostic.scope {
        AffectedScope::Global => "global".to_owned(),
        AffectedScope::Repository { repository } => format!("repository={repository}"),
        AffectedScope::Item { repository, item } => {
            format!("repository={repository} item={item}")
        }
    };
    let field = diagnostic
        .field
        .as_deref()
        .map(|value| format!(" field={value}"))
        .unwrap_or_default();
    format!(
        "{} [{}] {} at {}{}{}",
        diagnostic.code,
        diagnostic.stage.label(),
        scope,
        diagnostic.authority_path,
        field,
        ""
    )
}

#[cfg(test)]
mod diagnostics_tests;
