//! Source declaration parsing from pinned snapshots.
//!
//! Owner-selected declaration files inside each immutable pinned snapshot
//! are read through the typed read-only source authority root (no-follow
//! relative resolution).  Every record carries its source identity, pinned
//! revision, declaration path, and ordered field pairs; declaration order
//! and provenance are preserved.  Parse errors identify the exact source
//! and entry.  No destination access or content extraction happens here.

#![allow(dead_code)]

use super::snapshot::{RevisionId, SourceId};
use std::{error::Error, fmt};

/// Declaration file protocol version.
pub const DECLARATION_VERSION: &str = "omnirepo-declarations-v1";

/// One parsed source declaration record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDeclaration {
    pub source: SourceId,
    pub revision: RevisionId,
    pub path: String,
    pub fields: Vec<(String, String)>,
    /// The declaration's file and entry index (order + provenance).
    pub provenance: String,
}

/// Parsing failures; every error names the source and the entry.
#[derive(Debug)]
pub enum DeclarationsError {
    UnsupportedVersion {
        source: SourceId,
        file: String,
        version: String,
    },
    MalformedRecord {
        source: SourceId,
        file: String,
        entry: u64,
        reason: String,
    },
    Identity {
        source: String,
        file: String,
        entry: u64,
        reason: String,
    },
}

impl fmt::Display for DeclarationsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion {
                source,
                file,
                version,
            } => write!(
                formatter,
                "source {} declaration {} carries unsupported version {version:?}",
                source.as_str(),
                file
            ),
            Self::MalformedRecord {
                source,
                file,
                entry,
                reason,
            } => write!(
                formatter,
                "source {} declaration {} entry {entry} is malformed: {reason}",
                source.as_str(),
                file
            ),
            Self::Identity {
                source,
                file,
                entry,
                reason,
            } => write!(
                formatter,
                "source {source} declaration {file} entry {entry} has an invalid identity: {reason}"
            ),
        }
    }
}
impl Error for DeclarationsError {}

/// Parse the owner-selected declaration files' content.
///
/// The reader (composition side) resolves each file through the typed
/// read-only source authority; this pure parser consumes the exact bytes.
/// Records preserve file and entry order across files.
pub fn parse_declarations(
    source: &SourceId,
    revision: &RevisionId,
    files: &[(&str, String)],
) -> Result<Vec<SourceDeclaration>, DeclarationsError> {
    let mut declarations = Vec::new();
    for (file, content) in files {
        let mut lines = content.lines();
        let header = lines.next().unwrap_or_default().trim();
        if header != DECLARATION_VERSION {
            return Err(DeclarationsError::UnsupportedVersion {
                source: source.clone(),
                file: (*file).to_owned(),
                version: header.to_owned(),
            });
        }
        for (index, line) in lines.enumerate() {
            let entry = (index + 1) as u64;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            declarations.push(parse_record(source, revision, file, entry, line).map_err(
                |reason| DeclarationsError::MalformedRecord {
                    source: source.clone(),
                    file: (*file).to_owned(),
                    entry,
                    reason,
                },
            )?);
        }
    }
    Ok(declarations)
}

fn parse_record(
    source: &SourceId,
    revision: &RevisionId,
    file: &str,
    entry: u64,
    line: &str,
) -> Result<SourceDeclaration, String> {
    let mut fields = Vec::new();
    let mut declared_source: Option<String> = None;
    let mut declared_revision: Option<String> = None;
    let mut declared_path: Option<String> = None;
    for token in line.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!("field {token:?} is not key=value"));
        };
        if key.is_empty() || value.is_empty() {
            return Err(format!("field {token:?} has an empty key or value"));
        }
        match key {
            "source" => declared_source = Some(value.to_owned()),
            "revision" => declared_revision = Some(value.to_owned()),
            "path" => declared_path = Some(value.to_owned()),
            _ => fields.push((key.to_owned(), value.to_owned())),
        }
    }
    let declared_source = declared_source.ok_or_else(|| "missing source field".to_owned())?;
    let declared_revision = declared_revision.ok_or_else(|| "missing revision field".to_owned())?;
    let declared_path = declared_path.ok_or_else(|| "missing path field".to_owned())?;
    if declared_source != source.as_str() {
        return Err(format!(
            "declared source {declared_source:?} does not match the pinned source {:?}",
            source.as_str()
        ));
    }
    if declared_revision != revision.as_str() {
        return Err(format!(
            "declared revision {declared_revision:?} does not match the pinned revision {:?}",
            revision.as_str()
        ));
    }
    Ok(SourceDeclaration {
        source: source.clone(),
        revision: revision.clone(),
        path: declared_path,
        fields,
        provenance: format!("{}:{}", file, entry),
    })
}
