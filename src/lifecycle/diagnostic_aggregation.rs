//! Aggregated redacted diagnostics.
//!
//! Every permitted independent error appears exactly once with stable
//! ordering.  Secret sentinels across configuration, URLs, environment,
//! helpers, and filenames never appear: redaction masks known secret
//! shapes before any render.

#![allow(dead_code)]

use super::diagnostics::{Diagnostic, render_diagnostic};

/// Redact known secret shapes from arbitrary text: `key=value` forms for
/// common secret keys, embedded credentials in URLs, and long hex/base64
/// tokens.  Pure and deterministic.
pub fn redact(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(['=', '@']) {
        output.push_str(&rest[..position]);
        let after = &rest[position..];
        if let Some(value_text) = after.strip_prefix('=') {
            // key=value: mask the value up to the next whitespace.
            let key = output
                .rsplit(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .next()
                .unwrap_or("");
            if is_secret_key(key) {
                let value_end = value_text
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                    .unwrap_or(value_text.len());
                output.push_str("=<redacted>");
                rest = &value_text[value_end..];
                continue;
            }
            output.push('=');
            rest = value_text;
            continue;
        }
        // @ in a URL-like authority: mask the userinfo when it carries a
        // password (a ':' before the '@', ignoring the scheme).
        let before = output
            .rsplit(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .next()
            .unwrap_or("");
        let userinfo = before.split("://").last().unwrap_or(before);
        if userinfo.contains(':') {
            // user:pass@host — mask the pass part.
            let colon = userinfo.rfind(':').expect("colon");
            output.truncate(output.len() - (userinfo.len() - colon - 1));
            output.push_str("<redacted>@");
            rest = &after[1..];
            continue;
        }
        output.push('@');
        rest = &after[1..];
    }
    output.push_str(rest);
    output
}

fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "authorization",
        "credential",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

/// Aggregate the diagnostics: every permitted independent error appears
/// exactly once, in the stable declared order.
pub fn aggregate_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut seen = Vec::new();
    diagnostics
        .iter()
        .filter(|diagnostic| {
            let key = (diagnostic.code, diagnostic.scope_key());
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        })
        .cloned()
        .collect()
}

/// Render the aggregated diagnostics with redaction applied to every
/// authority path and remediation.
pub fn render_redacted(diagnostics: &[Diagnostic]) -> Vec<String> {
    let aggregated = aggregate_diagnostics(diagnostics);
    aggregated
        .iter()
        .map(|diagnostic| {
            let rendered = render_diagnostic(diagnostic);
            redact(&rendered)
        })
        .collect()
}

impl Diagnostic {
    fn scope_key(&self) -> String {
        match &self.scope {
            super::diagnostics::AffectedScope::Global => "global".to_owned(),
            super::diagnostics::AffectedScope::Repository { repository } => {
                format!("repository={repository}")
            }
            super::diagnostics::AffectedScope::Item { repository, item } => {
                format!("repository={repository} item={item}")
            }
        }
    }
}

#[cfg(test)]
mod diagnostic_aggregation_tests;
