//! Deterministic coverage attribution for the repository's product source.
//!
//! The threshold and toolchain authority remains `scripts/coverage.sh`.  This
//! module only parses the report produced by that gate and attributes
//! uncovered product source to one row in the canonical traceability matrix.
//! The ownership file is a projection of that matrix, not a second taxonomy.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::Visitor};
use serde_json::{Map, Value, de::Deserializer};

pub const REPORT_SCHEMA: &str = "omnirepo.coverage-ownership-report.v1";
pub const OWNERSHIP_SCHEMA: &str = "omnirepo.coverage-ownership.v1";
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_REPORT_BYTES: usize = 1024 * 1024;
const MAX_FIELD_BYTES: usize = 16 * 1024;
const MAX_SOURCE_FILES: usize = 512;
const MAX_TREE_DEPTH: usize = 32;

/// A bounded error from coverage parsing or attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageError {
    InputTooLarge {
        kind: &'static str,
        limit: usize,
    },
    Json {
        context: &'static str,
        message: String,
    },
    Schema {
        context: String,
        message: String,
    },
    Lcov {
        line: usize,
        message: String,
    },
    Io {
        path: PathBuf,
        message: String,
    },
    Ownership {
        path: String,
        message: String,
    },
    ReportTooLarge {
        limit: usize,
    },
}

impl fmt::Display for CoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge { kind, limit } => {
                write!(formatter, "{kind} exceeds the {limit}-byte input limit")
            }
            Self::Json { context, message } => {
                write!(formatter, "{context} JSON is invalid: {message}")
            }
            Self::Schema { context, message } => {
                write!(formatter, "{context} schema is invalid: {message}")
            }
            Self::Lcov { line, message } => {
                write!(formatter, "LCOV line {line} is invalid: {message}")
            }
            Self::Io { path, message } => {
                write!(formatter, "cannot inspect {}: {message}", path.display())
            }
            Self::Ownership { path, message } => {
                write!(formatter, "coverage ownership {path}: {message}")
            }
            Self::ReportTooLarge { limit } => {
                write!(formatter, "coverage ownership report exceeds {limit} bytes")
            }
        }
    }
}

impl std::error::Error for CoverageError {}

/// A matrix identity copied into the coverage report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OwnerIdentity {
    pub row_id: String,
    pub case_id: String,
    pub evidence_id: String,
    pub primary_owner: String,
}

/// Coverage totals for one product source file or the complete product.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct CoverageTotals {
    pub lines_total: u64,
    pub lines_covered: u64,
    pub functions_total: u64,
    pub functions_covered: u64,
    pub regions_total: u64,
    pub regions_covered: u64,
}

/// An uncovered executable line with its exact owning row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UncoveredLine {
    pub path: String,
    pub line: u64,
    pub owner: OwnerIdentity,
}

/// An uncovered function with its declaration line and owning row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UncoveredFunction {
    pub path: String,
    pub line: u64,
    pub name: String,
    pub owner: OwnerIdentity,
}

/// An uncovered LCOV branch/region with its exact location and owning row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UncoveredRegion {
    pub path: String,
    pub line: u64,
    pub block: String,
    pub branch: String,
    pub owner: OwnerIdentity,
}

/// Coverage for one product source path.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceCoverage {
    pub path: String,
    pub owner: OwnerIdentity,
    pub totals: CoverageTotals,
    pub uncovered_lines: Vec<UncoveredLine>,
    pub uncovered_functions: Vec<UncoveredFunction>,
    pub uncovered_regions: Vec<UncoveredRegion>,
}

/// The bounded, deterministic coverage attribution result.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CoverageReport {
    pub schema: &'static str,
    pub scope: &'static str,
    pub totals: CoverageTotals,
    pub sources: Vec<SourceCoverage>,
}

impl CoverageReport {
    /// Serialize the report while enforcing the retained-evidence bound.
    pub fn json(&self) -> Result<String, CoverageError> {
        let json = serde_json::to_string(self).map_err(|error| CoverageError::Json {
            context: "coverage report",
            message: error.to_string(),
        })?;
        if json.len() > MAX_REPORT_BYTES {
            return Err(CoverageError::ReportTooLarge {
                limit: MAX_REPORT_BYTES,
            });
        }
        Ok(json)
    }
}

/// Read the repository source and report files, then attribute product gaps.
pub fn attribute_repository(
    repository_root: &Path,
    lcov_path: &Path,
    matrix_path: &Path,
    ownership_path: &Path,
) -> Result<CoverageReport, CoverageError> {
    let lcov = read_bounded(lcov_path, "LCOV")?;
    let matrix = read_bounded(matrix_path, "traceability matrix")?;
    let ownership = read_bounded(ownership_path, "coverage ownership")?;
    let source_paths = discover_product_sources(repository_root)?;
    attribute_lcov_with_sources(
        &lcov,
        &matrix,
        &ownership,
        Some(&source_paths),
        Some(repository_root),
    )
}

/// Attribute an LCOV string against supplied JSON identities.
///
/// This filesystem-free seam is used by deterministic contract tests.  The
/// optional source list is supplied by [`attribute_repository`] for the full
/// gate, where every runtime source must have exactly one mapping.
pub fn attribute_lcov(
    lcov: &str,
    matrix_json: &str,
    ownership_json: &str,
) -> Result<CoverageReport, CoverageError> {
    attribute_lcov_with_sources(lcov, matrix_json, ownership_json, None, None)
}

fn attribute_lcov_with_sources(
    lcov: &str,
    matrix_json: &str,
    ownership_json: &str,
    source_paths: Option<&BTreeSet<String>>,
    repository_root: Option<&Path>,
) -> Result<CoverageReport, CoverageError> {
    enforce_input_limit(lcov, "LCOV")?;
    enforce_input_limit(matrix_json, "traceability matrix")?;
    enforce_input_limit(ownership_json, "coverage ownership")?;

    let matrix = parse_matrix(matrix_json)?;
    let ownership = parse_ownership(ownership_json, &matrix)?;
    if let Some(source_paths) = source_paths {
        let mapped = ownership.keys().cloned().collect::<BTreeSet<_>>();
        if mapped != *source_paths {
            let missing = source_paths
                .difference(&mapped)
                .cloned()
                .collect::<Vec<_>>();
            let extra = mapped.difference(source_paths).cloned().collect::<Vec<_>>();
            return Err(CoverageError::Ownership {
                path: "<source-tree>".to_owned(),
                message: format!(
                    "source mapping does not exactly match product sources; missing={missing:?}, extra={extra:?}"
                ),
            });
        }
    }

    let records = parse_lcov(lcov)?;
    let mut sources = Vec::new();
    let mut seen_product_paths = BTreeSet::new();
    for mut record in records {
        if !is_product_source_path(&record.path) {
            if let Some(repository_root) = repository_root {
                if let Some(relative) = absolute_source_path(&record.path, repository_root)? {
                    record.path = relative;
                }
            }
        }
        if !is_product_source_path(&record.path) {
            continue;
        }
        let path = normalize_source_path(&record.path)?;
        if !seen_product_paths.insert(path.clone()) {
            return Err(CoverageError::Lcov {
                line: record.first_line,
                message: format!("duplicate product source record {path:?}"),
            });
        }
        let owner = ownership
            .get(&path)
            .ok_or_else(|| CoverageError::Ownership {
                path: path.clone(),
                message: "product source has no canonical matrix owner".to_owned(),
            })?;
        sources.push(materialize_source(record, owner.clone()));
    }

    if let Some(source_paths) = source_paths {
        let missing_records = source_paths
            .difference(&seen_product_paths)
            .cloned()
            .collect::<Vec<_>>();
        if !missing_records.is_empty() {
            let Some(repository_root) = repository_root else {
                return Err(CoverageError::Ownership {
                    path: "<source-tree>".to_owned(),
                    message: "missing LCOV records require a repository root".to_owned(),
                });
            };
            for path in missing_records {
                let owner = ownership
                    .get(&path)
                    .ok_or_else(|| CoverageError::Ownership {
                        path: path.clone(),
                        message: "product source has no canonical matrix owner".to_owned(),
                    })?;
                if !is_declaration_only_facade(repository_root, &path)? {
                    return Err(CoverageError::Ownership {
                        path,
                        message:
                            "product source has no LCOV record and is not a declaration-only facade"
                                .to_owned(),
                    });
                }
                sources.push(SourceCoverage {
                    path,
                    owner: owner.clone(),
                    totals: CoverageTotals::default(),
                    uncovered_lines: Vec::new(),
                    uncovered_functions: Vec::new(),
                    uncovered_regions: Vec::new(),
                });
            }
        }
    }

    sources.sort_by(|left, right| left.path.cmp(&right.path));
    let mut totals = CoverageTotals::default();
    for source in &sources {
        add_totals(&mut totals, &source.totals);
    }
    Ok(CoverageReport {
        schema: REPORT_SCHEMA,
        scope: "publishable-product-src",
        totals,
        sources,
    })
}

fn add_totals(target: &mut CoverageTotals, source: &CoverageTotals) {
    target.lines_total += source.lines_total;
    target.lines_covered += source.lines_covered;
    target.functions_total += source.functions_total;
    target.functions_covered += source.functions_covered;
    target.regions_total += source.regions_total;
    target.regions_covered += source.regions_covered;
}

fn is_declaration_only_facade(repository_root: &Path, path: &str) -> Result<bool, CoverageError> {
    let source_path = repository_root.join(path);
    let source = read_bounded(&source_path, "product source")?;
    Ok(parse_declaration_only_facade(&source))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FacadeToken {
    Identifier(String),
    Symbol(u8),
    Literal,
}

fn parse_declaration_only_facade(source: &str) -> bool {
    let Some(tokens) = lex_facade_source(source) else {
        return false;
    };
    let mut position = 0;
    let mut declarations = 0;
    while position < tokens.len() {
        if !consume_attributes(&tokens, &mut position) {
            return false;
        }
        if !consume_visibility(&tokens, &mut position) {
            return false;
        }
        let Some(FacadeToken::Identifier(kind)) = tokens.get(position) else {
            return false;
        };
        match kind.as_str() {
            "mod" => {
                position += 1;
                if !matches!(tokens.get(position), Some(FacadeToken::Identifier(_))) {
                    return false;
                }
                position += 1;
                if !matches!(tokens.get(position), Some(FacadeToken::Symbol(b';'))) {
                    return false;
                }
                position += 1;
                declarations += 1;
            }
            "use" => {
                position += 1;
                if !consume_use(&tokens, &mut position) {
                    return false;
                }
                declarations += 1;
            }
            _ => return false,
        }
    }
    declarations > 0
}

fn consume_attributes(tokens: &[FacadeToken], position: &mut usize) -> bool {
    while matches!(tokens.get(*position), Some(FacadeToken::Symbol(b'#'))) {
        *position += 1;
        if matches!(tokens.get(*position), Some(FacadeToken::Symbol(b'!'))) {
            *position += 1;
        }
        if !consume_balanced(tokens, position, b'[', b']') {
            return false;
        }
    }
    true
}

fn consume_visibility(tokens: &[FacadeToken], position: &mut usize) -> bool {
    if !matches!(
        tokens.get(*position),
        Some(FacadeToken::Identifier(identifier)) if identifier == "pub"
    ) {
        return true;
    }
    *position += 1;
    if matches!(tokens.get(*position), Some(FacadeToken::Symbol(b'('))) {
        return consume_balanced(tokens, position, b'(', b')');
    }
    true
}

fn consume_balanced(
    tokens: &[FacadeToken],
    position: &mut usize,
    opening: u8,
    closing: u8,
) -> bool {
    if !matches!(
        tokens.get(*position),
        Some(FacadeToken::Symbol(symbol)) if *symbol == opening
    ) {
        return false;
    }
    let mut delimiters = vec![closing];
    *position += 1;
    while let Some(token) = tokens.get(*position) {
        match token {
            FacadeToken::Symbol(symbol) if matches!(symbol, b'(' | b'[' | b'{') => {
                delimiters.push(match symbol {
                    b'(' => b')',
                    b'[' => b']',
                    b'{' => b'}',
                    _ => unreachable!(),
                });
            }
            FacadeToken::Symbol(symbol) if matches!(symbol, b')' | b']' | b'}') => {
                if delimiters.pop() != Some(*symbol) {
                    return false;
                }
                if delimiters.is_empty() {
                    *position += 1;
                    return true;
                }
            }
            _ => {}
        }
        *position += 1;
    }
    false
}

fn consume_use(tokens: &[FacadeToken], position: &mut usize) -> bool {
    let start = *position;
    let mut delimiters = Vec::new();
    let mut has_identifier = false;
    while let Some(token) = tokens.get(*position) {
        match token {
            FacadeToken::Symbol(b';') if delimiters.is_empty() => {
                *position += 1;
                return *position > start + 1 && has_identifier;
            }
            FacadeToken::Symbol(symbol) if matches!(symbol, b'(' | b'[' | b'{') => {
                delimiters.push(match symbol {
                    b'(' => b')',
                    b'[' => b']',
                    b'{' => b'}',
                    _ => unreachable!(),
                });
            }
            FacadeToken::Symbol(symbol)
                if matches!(symbol, b')' | b']' | b'}') && delimiters.pop() != Some(*symbol) =>
            {
                return false;
            }
            FacadeToken::Identifier(_) => has_identifier = true,
            _ => {}
        }
        *position += 1;
    }
    false
}

fn lex_facade_source(source: &str) -> Option<Vec<FacadeToken>> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        match bytes[position] {
            whitespace if whitespace.is_ascii_whitespace() => position += 1,
            b'/' if bytes.get(position + 1) == Some(&b'/') => {
                position += 2;
                while position < bytes.len() && bytes[position] != b'\n' {
                    position += 1;
                }
            }
            b'/' if bytes.get(position + 1) == Some(&b'*') => {
                position += 2;
                let mut depth = 1usize;
                while position < bytes.len() && depth > 0 {
                    if bytes.get(position..position + 2) == Some(b"/*") {
                        depth += 1;
                        position += 2;
                    } else if bytes.get(position..position + 2) == Some(b"*/") {
                        depth -= 1;
                        position += 2;
                    } else {
                        position += 1;
                    }
                }
                if depth != 0 {
                    return None;
                }
            }
            b'r' | b'b' if raw_string_prefix(bytes, position).is_some() => {
                let end = consume_raw_string(bytes, position)?;
                tokens.push(FacadeToken::Literal);
                position = end;
            }
            b'b' if bytes.get(position + 1) == Some(&b'\"') => {
                let end = consume_quoted(bytes, position + 1, b'\"')?;
                tokens.push(FacadeToken::Literal);
                position = end;
            }
            b'\"' => {
                let end = consume_quoted(bytes, position, b'\"')?;
                tokens.push(FacadeToken::Literal);
                position = end;
            }
            b'\'' => {
                if let Some(end) = consume_quoted(bytes, position, b'\'') {
                    tokens.push(FacadeToken::Literal);
                    position = end;
                } else {
                    tokens.push(FacadeToken::Symbol(b'\''));
                    position += 1;
                }
            }
            byte if is_identifier_start(byte) => {
                let start = position;
                position += 1;
                while position < bytes.len() && is_identifier_continue(bytes[position]) {
                    position += 1;
                }
                let identifier = std::str::from_utf8(&bytes[start..position])
                    .ok()?
                    .to_owned();
                tokens.push(FacadeToken::Identifier(identifier));
            }
            byte if byte.is_ascii_digit() => {
                position += 1;
                while position < bytes.len()
                    && (bytes[position].is_ascii_alphanumeric() || bytes[position] == b'_')
                {
                    position += 1;
                }
                tokens.push(FacadeToken::Literal);
            }
            symbol => {
                tokens.push(FacadeToken::Symbol(symbol));
                position += 1;
            }
        }
    }
    Some(tokens)
}

fn raw_string_prefix(bytes: &[u8], position: usize) -> Option<usize> {
    let mut cursor = position;
    if bytes.get(cursor) == Some(&b'b') {
        cursor += 1;
        if bytes.get(cursor) != Some(&b'r') {
            return None;
        }
    } else if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    cursor += 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'\"')).then_some(cursor - position + 1)
}

fn consume_raw_string(bytes: &[u8], position: usize) -> Option<usize> {
    let prefix = raw_string_prefix(bytes, position)?;
    let mut cursor = position + prefix;
    let mut hashes = 0;
    let mut marker = position + 1;
    if bytes.get(position) == Some(&b'b') {
        marker += 1;
    }
    while bytes.get(marker) == Some(&b'#') {
        hashes += 1;
        marker += 1;
    }
    while cursor < bytes.len() {
        if bytes.get(cursor) == Some(&b'\"')
            && bytes.get(cursor + 1..cursor + 1 + hashes) == Some(&bytes[marker - hashes..marker])
        {
            return Some(cursor + hashes + 1);
        }
        cursor += 1;
    }
    None
}

fn consume_quoted(bytes: &[u8], position: usize, quote: u8) -> Option<usize> {
    let mut cursor = position + 1;
    let mut escaped = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == quote {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn materialize_source(record: LcovRecord, owner: OwnerIdentity) -> SourceCoverage {
    let path = record.path;
    let mut uncovered_lines = record
        .lines
        .into_iter()
        .filter(|line| line.count == 0)
        .map(|line| UncoveredLine {
            path: path.clone(),
            line: line.line,
            owner: owner.clone(),
        })
        .collect::<Vec<_>>();
    let mut uncovered_functions = record
        .functions
        .into_iter()
        .filter(|function| function.count == 0)
        .map(|function| UncoveredFunction {
            path: path.clone(),
            line: function.line,
            name: function.name,
            owner: owner.clone(),
        })
        .collect::<Vec<_>>();
    let mut uncovered_regions = record
        .regions
        .into_iter()
        .filter(|region| region.taken == "-")
        .map(|region| UncoveredRegion {
            path: path.clone(),
            line: region.line,
            block: region.block,
            branch: region.branch,
            owner: owner.clone(),
        })
        .collect::<Vec<_>>();
    uncovered_lines.sort_by_key(|line| line.line);
    uncovered_functions
        .sort_by(|left, right| (left.line, &left.name).cmp(&(right.line, &right.name)));
    uncovered_regions.sort_by(|left, right| {
        (left.line, &left.block, &left.branch).cmp(&(right.line, &right.block, &right.branch))
    });
    SourceCoverage {
        path,
        owner,
        totals: record.totals,
        uncovered_lines,
        uncovered_functions,
        uncovered_regions,
    }
}

fn read_bounded(path: &Path, kind: &'static str) -> Result<String, CoverageError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| CoverageError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoverageError::Io {
            path: path.to_path_buf(),
            message: "expected a regular non-symlink file".to_owned(),
        });
    }
    let bytes = fs::read(path).map_err(|error| CoverageError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    enforce_input_limit(&bytes, kind)?;
    String::from_utf8(bytes).map_err(|error| CoverageError::Io {
        path: path.to_path_buf(),
        message: format!("not UTF-8: {error}"),
    })
}

fn enforce_input_limit(input: impl AsRef<[u8]>, kind: &'static str) -> Result<(), CoverageError> {
    if input.as_ref().len() > MAX_INPUT_BYTES {
        return Err(CoverageError::InputTooLarge {
            kind,
            limit: MAX_INPUT_BYTES,
        });
    }
    Ok(())
}

fn discover_product_sources(repository_root: &Path) -> Result<BTreeSet<String>, CoverageError> {
    let source_root = repository_root.join("src");
    let metadata = fs::symlink_metadata(&source_root).map_err(|error| CoverageError::Io {
        path: source_root.clone(),
        message: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoverageError::Io {
            path: source_root,
            message: "expected a regular non-symlink source directory".to_owned(),
        });
    }
    let mut paths = BTreeSet::new();
    discover_product_sources_inner(repository_root, Path::new("src"), 0, &mut paths)?;
    if paths.is_empty() {
        return Err(CoverageError::Ownership {
            path: "src".to_owned(),
            message: "product source tree contains no runtime Rust files".to_owned(),
        });
    }
    Ok(paths)
}

fn discover_product_sources_inner(
    repository_root: &Path,
    relative: &Path,
    depth: usize,
    paths: &mut BTreeSet<String>,
) -> Result<(), CoverageError> {
    if depth > MAX_TREE_DEPTH {
        return Err(CoverageError::Io {
            path: repository_root.join(relative),
            message: "source traversal depth limit exceeded".to_owned(),
        });
    }
    let directory = repository_root.join(relative);
    let entries = fs::read_dir(&directory).map_err(|error| CoverageError::Io {
        path: directory.clone(),
        message: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| CoverageError::Io {
            path: directory.clone(),
            message: error.to_string(),
        })?;
        let name = entry.file_name();
        let child_relative = relative.join(&name);
        let child = repository_root.join(&child_relative);
        let metadata = fs::symlink_metadata(&child).map_err(|error| CoverageError::Io {
            path: child.clone(),
            message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(CoverageError::Io {
                path: child,
                message: "symlinked product source is not admissible".to_owned(),
            });
        }
        if metadata.is_dir() {
            discover_product_sources_inner(repository_root, &child_relative, depth + 1, paths)?;
        } else if metadata.is_file()
            && child_relative
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("rs")
            && is_runtime_source_path(&child_relative)
        {
            if paths.len() == MAX_SOURCE_FILES {
                return Err(CoverageError::Io {
                    path: child,
                    message: "source file-count limit exceeded".to_owned(),
                });
            }
            paths.insert(path_to_slash(&child_relative));
        }
    }
    Ok(())
}

fn is_runtime_source_path(path: &Path) -> bool {
    if path
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new("tests"))
    {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name != "tests.rs" && !name.ends_with("_tests.rs")
}

fn is_product_source_path(path: &str) -> bool {
    path.starts_with("src/") && path.ends_with(".rs") && is_runtime_source_path(Path::new(path))
}

fn path_to_slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_source_path(path: &str) -> Result<String, CoverageError> {
    if path.len() > MAX_FIELD_BYTES
        || path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains("//")
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !is_product_source_path(path)
    {
        return Err(CoverageError::Ownership {
            path: path.to_owned(),
            message: "source path is not a normalized product-relative Rust path".to_owned(),
        });
    }
    Ok(path.to_owned())
}

fn absolute_source_path(
    raw: &str,
    repository_root: &Path,
) -> Result<Option<String>, CoverageError> {
    let normalized_separators = raw.replace('\\', "/");
    let candidate = Path::new(&normalized_separators);
    if !candidate.is_absolute() {
        return Ok(None);
    }
    let root = repository_root
        .canonicalize()
        .map_err(|error| CoverageError::Io {
            path: repository_root.to_path_buf(),
            message: error.to_string(),
        })?;
    let relative = candidate
        .strip_prefix(&root)
        .map_err(|_| CoverageError::Ownership {
            path: raw.to_owned(),
            message: "absolute LCOV source is outside the repository root".to_owned(),
        })?;
    let relative = path_to_slash(relative);
    Ok(Some(normalize_source_path(&relative)?))
}

#[derive(Debug, Clone)]
struct MatrixRow {
    row_id: String,
    case_id: String,
    evidence_id: String,
    primary_owner: String,
}

fn parse_matrix(source: &str) -> Result<BTreeMap<String, MatrixRow>, CoverageError> {
    let root = parse_json(source, "traceability matrix")?;
    let root_object = object(&root, "traceability matrix")?;
    if string_field(root_object, "schema", "traceability matrix")?
        != "omnirepo.traceability-matrix.v1"
    {
        return Err(schema_error(
            "traceability matrix",
            "schema must be omnirepo.traceability-matrix.v1",
        ));
    }
    if string_field(root_object, "status", "traceability matrix")? != "canonical" {
        return Err(schema_error(
            "traceability matrix",
            "status must be canonical",
        ));
    }
    let rows = array_field(root_object, "rows", "traceability matrix")?;
    let mut result = BTreeMap::new();
    for (index, row_value) in rows.iter().enumerate() {
        let row_context = format!("traceability matrix row {index}");
        let row = object(row_value, &row_context)?;
        let identity = MatrixRow {
            row_id: string_field(row, "id", &row_context)?.to_owned(),
            case_id: string_field(row, "case_id", &row_context)?.to_owned(),
            evidence_id: string_field(row, "evidence_id", &row_context)?.to_owned(),
            primary_owner: string_field(row, "primary_owner", &row_context)?.to_owned(),
        };
        if result.insert(identity.row_id.clone(), identity).is_some() {
            return Err(schema_error(&row_context, "duplicate row id"));
        }
    }
    if result.is_empty() {
        return Err(schema_error(
            "traceability matrix",
            "rows must not be empty",
        ));
    }
    Ok(result)
}

fn parse_ownership(
    source: &str,
    matrix: &BTreeMap<String, MatrixRow>,
) -> Result<BTreeMap<String, OwnerIdentity>, CoverageError> {
    let root = parse_json(source, "coverage ownership")?;
    let root_object = object(&root, "coverage ownership")?;
    require_exact_keys(
        root_object,
        "coverage ownership",
        &["schema", "status", "entries"],
    )?;
    if string_field(root_object, "schema", "coverage ownership")? != OWNERSHIP_SCHEMA {
        return Err(schema_error(
            "coverage ownership",
            "schema must be omnirepo.coverage-ownership.v1",
        ));
    }
    if string_field(root_object, "status", "coverage ownership")? != "canonical-projection" {
        return Err(schema_error(
            "coverage ownership",
            "status must be canonical-projection",
        ));
    }
    let entries = array_field(root_object, "entries", "coverage ownership")?;
    let mut result = BTreeMap::new();
    for (index, entry_value) in entries.iter().enumerate() {
        let context = format!("coverage ownership entry {index}");
        let entry = object(entry_value, &context)?;
        require_exact_keys(entry, &context, &["path", "row_id"])?;
        let path = normalize_source_path(string_field(entry, "path", &context)?)?;
        let row_id = string_field(entry, "row_id", &context)?.to_owned();
        let matrix_row = matrix
            .get(&row_id)
            .ok_or_else(|| CoverageError::Ownership {
                path: path.clone(),
                message: format!("row {row_id:?} is absent from the canonical matrix"),
            })?;
        let owner = OwnerIdentity {
            row_id: row_id.clone(),
            case_id: matrix_row.case_id.clone(),
            evidence_id: matrix_row.evidence_id.clone(),
            primary_owner: matrix_row.primary_owner.clone(),
        };
        if result.insert(path.clone(), owner).is_some() {
            return Err(CoverageError::Ownership {
                path,
                message: "duplicate source path mapping".to_owned(),
            });
        }
    }
    if result.is_empty() {
        return Err(schema_error(
            "coverage ownership",
            "entries must not be empty",
        ));
    }
    Ok(result)
}

fn schema_error(context: &str, message: &str) -> CoverageError {
    CoverageError::Schema {
        context: context.to_owned(),
        message: message.to_owned(),
    }
}

fn require_exact_keys(
    object: &Map<String, Value>,
    context: &str,
    allowed: &[&str],
) -> Result<(), CoverageError> {
    if let Some(unknown) = object
        .keys()
        .find(|key| !allowed.iter().any(|allowed_key| allowed_key == key))
    {
        return Err(schema_error(context, &format!("unknown field {unknown:?}")));
    }
    Ok(())
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, CoverageError> {
    value
        .as_object()
        .ok_or_else(|| schema_error(context, "expected an object"))
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a str, CoverageError> {
    let value = object
        .get(name)
        .ok_or_else(|| schema_error(context, &format!("missing field {name:?}")))?;
    let value = value
        .as_str()
        .ok_or_else(|| schema_error(context, &format!("field {name:?} must be a string")))?;
    if value.is_empty() || value.len() > MAX_FIELD_BYTES || value.chars().any(char::is_control) {
        return Err(schema_error(
            context,
            &format!("field {name:?} is empty, oversized, or contains control text"),
        ));
    }
    Ok(value)
}

fn array_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a Vec<Value>, CoverageError> {
    object
        .get(name)
        .ok_or_else(|| schema_error(context, &format!("missing field {name:?}")))?
        .as_array()
        .ok_or_else(|| schema_error(context, &format!("field {name:?} must be an array")))
}

fn parse_json(source: &str, context: &'static str) -> Result<Value, CoverageError> {
    let mut deserializer = Deserializer::from_str(source);
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| CoverageError::Json {
            context,
            message: error.to_string(),
        })?
        .0;
    deserializer.end().map_err(|error| CoverageError::Json {
        context,
        message: error.to_string(),
    })?;
    validate_json_depth(&value, 0, context)?;
    Ok(value)
}

fn validate_json_depth(
    value: &Value,
    depth: usize,
    context: &'static str,
) -> Result<(), CoverageError> {
    if depth > MAX_TREE_DEPTH {
        return Err(schema_error(
            context,
            "JSON nesting depth exceeds the bounded limit",
        ));
    }
    match value {
        Value::Array(values) => {
            if values.len() > MAX_SOURCE_FILES * 4 {
                return Err(schema_error(
                    context,
                    "JSON array exceeds the bounded limit",
                ));
            }
            for value in values {
                validate_json_depth(value, depth + 1, context)?;
            }
        }
        Value::Object(values) => {
            if values.len() > 128 {
                return Err(schema_error(
                    context,
                    "JSON object exceeds the bounded field limit",
                ));
            }
            for value in values.values() {
                validate_json_depth(value, depth + 1, context)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

/// A JSON value visitor that rejects duplicate object keys.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StrictVisitor;

        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value with unique object keys")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::Number(value.into())))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::Number(value.into())))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let number = serde_json::Number::from_f64(value)
                    .ok_or_else(|| E::custom("non-finite number is not valid JSON"))?;
                Ok(StrictValue(Value::Number(number)))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::String(value.to_owned())))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::String(value)))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::Null))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue(Value::Null))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate object key {key:?}"
                        )));
                    }
                    let value = map.next_value::<StrictValue>()?;
                    values.insert(key, value.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(StrictVisitor)
    }
}

#[derive(Debug, Clone)]
struct LcovRecord {
    path: String,
    first_line: usize,
    lines: Vec<LcovLine>,
    functions: Vec<LcovFunction>,
    regions: Vec<LcovRegion>,
    totals: CoverageTotals,
}

#[derive(Debug, Clone)]
struct LcovLine {
    line: u64,
    count: u64,
}

#[derive(Debug, Clone)]
struct LcovFunction {
    line: u64,
    name: String,
    count: u64,
}

#[derive(Debug, Clone)]
struct LcovRegion {
    line: u64,
    block: String,
    branch: String,
    taken: String,
}

fn parse_lcov(source: &str) -> Result<Vec<LcovRecord>, CoverageError> {
    let mut records = Vec::new();
    let mut current: Option<LcovBuilder> = None;
    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        if line == "end_of_record" {
            let builder = current
                .take()
                .ok_or_else(|| lcov_error(line_number, "end_of_record without SF"))?;
            records.push(builder.finish(line_number)?);
            continue;
        }
        let (tag, payload) = line.split_once(':').ok_or_else(|| CoverageError::Lcov {
            line: line_number,
            message: "record has no tag separator".to_owned(),
        })?;
        match tag {
            "TN" => {
                if payload.len() > MAX_FIELD_BYTES || payload.chars().any(char::is_control) {
                    return Err(lcov_error(
                        line_number,
                        "TN payload is oversized or contains control text",
                    ));
                }
            }
            "SF" => {
                if current.is_some() {
                    return Err(lcov_error(line_number, "SF started before end_of_record"));
                }
                if payload.is_empty() || payload.len() > MAX_FIELD_BYTES {
                    return Err(lcov_error(line_number, "SF path is empty or oversized"));
                }
                current = Some(LcovBuilder::new(payload.to_owned(), line_number));
            }
            "FN" => current_mut(&mut current, line_number)?.parse_function(payload, line_number)?,
            "FNDA" => {
                current_mut(&mut current, line_number)?.parse_function_data(payload, line_number)?
            }
            "FNF" => {
                current_mut(&mut current, line_number)?.set_function_total(payload, line_number)?
            }
            "FNH" => {
                current_mut(&mut current, line_number)?.set_function_hit(payload, line_number)?
            }
            "DA" => current_mut(&mut current, line_number)?.parse_line(payload, line_number)?,
            "LF" => current_mut(&mut current, line_number)?.set_line_total(payload, line_number)?,
            "LH" => current_mut(&mut current, line_number)?.set_line_hit(payload, line_number)?,
            "BRDA" => current_mut(&mut current, line_number)?.parse_region(payload, line_number)?,
            "BRF" => {
                current_mut(&mut current, line_number)?.set_region_total(payload, line_number)?
            }
            "BRH" => {
                current_mut(&mut current, line_number)?.set_region_hit(payload, line_number)?
            }
            other => {
                return Err(lcov_error(
                    line_number,
                    &format!("unsupported LCOV record {other:?}"),
                ));
            }
        }
    }
    if current.is_some() {
        return Err(lcov_error(
            source.lines().count().max(1),
            "missing end_of_record",
        ));
    }
    if records.is_empty() {
        return Err(lcov_error(1, "LCOV has no file records"));
    }
    Ok(records)
}

fn current_mut(
    current: &mut Option<LcovBuilder>,
    line: usize,
) -> Result<&mut LcovBuilder, CoverageError> {
    current
        .as_mut()
        .ok_or_else(|| lcov_error(line, "record appears before SF"))
}

fn lcov_error(line: usize, message: &str) -> CoverageError {
    CoverageError::Lcov {
        line,
        message: message.to_owned(),
    }
}

#[derive(Debug, Clone)]
struct LcovBuilder {
    path: String,
    first_line: usize,
    lines: Vec<LcovLine>,
    functions: Vec<(u64, String)>,
    function_data: Vec<(u64, String)>,
    regions: Vec<LcovRegion>,
    function_total: Option<u64>,
    function_hit: Option<u64>,
    line_total: Option<u64>,
    line_hit: Option<u64>,
    region_total: Option<u64>,
    region_hit: Option<u64>,
}

impl LcovBuilder {
    fn new(path: String, first_line: usize) -> Self {
        Self {
            path,
            first_line,
            lines: Vec::new(),
            functions: Vec::new(),
            function_data: Vec::new(),
            regions: Vec::new(),
            function_total: None,
            function_hit: None,
            line_total: None,
            line_hit: None,
            region_total: None,
            region_hit: None,
        }
    }

    fn parse_function(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        let (source_line, name) = payload
            .split_once(',')
            .ok_or_else(|| lcov_error(line, "FN requires line,name"))?;
        let source_line = parse_count(source_line, line, "FN line")?;
        validate_field(name, line, "FN name")?;
        self.functions.push((source_line, name.to_owned()));
        Ok(())
    }

    fn parse_function_data(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        let (count, name) = payload
            .split_once(',')
            .ok_or_else(|| lcov_error(line, "FNDA requires count,name"))?;
        let count = parse_count(count, line, "FNDA count")?;
        validate_field(name, line, "FNDA name")?;
        self.function_data.push((count, name.to_owned()));
        Ok(())
    }

    fn parse_line(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        let fields = payload.split(',').collect::<Vec<_>>();
        if fields.len() < 2 || fields.len() > 3 {
            return Err(lcov_error(line, "DA requires line,count[,checksum]"));
        }
        let source_line = parse_count(fields[0], line, "DA line")?;
        let count = parse_count(fields[1], line, "DA count")?;
        if self.lines.iter().any(|entry| entry.line == source_line) {
            return Err(lcov_error(line, "duplicate DA line"));
        }
        if fields.len() == 3 {
            validate_field(fields[2], line, "DA checksum")?;
        }
        self.lines.push(LcovLine {
            line: source_line,
            count,
        });
        Ok(())
    }

    fn parse_region(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        let fields = payload.split(',').collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(lcov_error(line, "BRDA requires line,block,branch,taken"));
        }
        let source_line = parse_count(fields[0], line, "BRDA line")?;
        for (field, name) in [
            (fields[1], "BRDA block"),
            (fields[2], "BRDA branch"),
            (fields[3], "BRDA taken"),
        ] {
            validate_field(field, line, name)?;
        }
        if self.regions.iter().any(|entry| {
            entry.line == source_line && entry.block == fields[1] && entry.branch == fields[2]
        }) {
            return Err(lcov_error(line, "duplicate BRDA branch"));
        }
        if fields[3] != "-" {
            let _ = parse_count(fields[3], line, "BRDA taken")?;
        }
        self.regions.push(LcovRegion {
            line: source_line,
            block: fields[1].to_owned(),
            branch: fields[2].to_owned(),
            taken: fields[3].to_owned(),
        });
        Ok(())
    }

    fn set_function_total(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        self.function_total = Some(set_summary(payload, line, "FNF", self.function_total)?);
        Ok(())
    }

    fn set_function_hit(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        self.function_hit = Some(set_summary(payload, line, "FNH", self.function_hit)?);
        Ok(())
    }

    fn set_line_total(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        self.line_total = Some(set_summary(payload, line, "LF", self.line_total)?);
        Ok(())
    }

    fn set_line_hit(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        self.line_hit = Some(set_summary(payload, line, "LH", self.line_hit)?);
        Ok(())
    }

    fn set_region_total(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        self.region_total = Some(set_summary(payload, line, "BRF", self.region_total)?);
        Ok(())
    }

    fn set_region_hit(&mut self, payload: &str, line: usize) -> Result<(), CoverageError> {
        self.region_hit = Some(set_summary(payload, line, "BRH", self.region_hit)?);
        Ok(())
    }

    fn finish(self, line: usize) -> Result<LcovRecord, CoverageError> {
        if function_data_names_match(&self.functions, &self.function_data).is_err() {
            return Err(lcov_error(line, "function records and summary disagree"));
        }
        let has_function_details = !self.functions.is_empty() || !self.function_data.is_empty();
        let function_total = match self.function_total {
            Some(total) => total,
            None if !has_function_details => 0,
            None => return Err(lcov_error(line, "missing FNF summary")),
        };
        let function_hit = match self.function_hit {
            Some(hit) => hit,
            None if !has_function_details => 0,
            None => return Err(lcov_error(line, "missing FNH summary")),
        };
        if function_hit > function_total {
            return Err(lcov_error(line, "FNH cannot exceed FNF"));
        }
        let line_total = self.line_total.unwrap_or(self.lines.len() as u64);
        let line_hit = self
            .line_hit
            .unwrap_or_else(|| self.lines.iter().filter(|entry| entry.count > 0).count() as u64);
        if line_hit > line_total {
            return Err(lcov_error(line, "LH cannot exceed LF"));
        }
        let region_total = self.region_total.unwrap_or(self.regions.len() as u64);
        let region_hit = self.region_hit.unwrap_or_else(|| {
            self.regions
                .iter()
                .filter(|entry| entry.taken != "-")
                .count() as u64
        });
        if region_hit > region_total {
            return Err(lcov_error(line, "BRH cannot exceed BRF"));
        }
        let functions = self
            .functions
            .into_iter()
            .map(|(line, name)| {
                let count = self
                    .function_data
                    .iter()
                    .find(|(_, data_name)| data_name == &name)
                    .map(|(count, _)| *count)
                    .unwrap_or(0);
                LcovFunction { line, name, count }
            })
            .collect();
        Ok(LcovRecord {
            path: self.path,
            first_line: self.first_line,
            lines: self.lines,
            functions,
            regions: self.regions,
            totals: CoverageTotals {
                lines_total: line_total,
                lines_covered: line_hit,
                functions_total: function_total,
                functions_covered: function_hit,
                regions_total: region_total,
                regions_covered: region_hit,
            },
        })
    }
}

fn function_data_names_match(
    functions: &[(u64, String)],
    data: &[(u64, String)],
) -> Result<(), ()> {
    if functions.len() != data.len() {
        return Err(());
    }
    let mut available = functions
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>();
    for (_, name) in data {
        let Some(position) = available.iter().position(|candidate| *candidate == name) else {
            return Err(());
        };
        available.remove(position);
    }
    Ok(())
}

fn set_summary(
    payload: &str,
    line: usize,
    tag: &str,
    previous: Option<u64>,
) -> Result<u64, CoverageError> {
    if previous.is_some() {
        return Err(lcov_error(line, &format!("duplicate {tag} summary")));
    }
    parse_count(payload, line, tag)
}

fn parse_count(value: &str, line: usize, field: &str) -> Result<u64, CoverageError> {
    if value.is_empty() || value.len() > MAX_FIELD_BYTES {
        return Err(lcov_error(line, &format!("{field} is empty or oversized")));
    }
    value
        .parse::<u64>()
        .map_err(|_| lcov_error(line, &format!("{field} must be an unsigned integer")))
}

fn validate_field(value: &str, line: usize, field: &str) -> Result<(), CoverageError> {
    if value.is_empty() || value.len() > MAX_FIELD_BYTES || value.chars().any(char::is_control) {
        return Err(lcov_error(
            line,
            &format!("{field} is empty, oversized, or contains control text"),
        ));
    }
    Ok(())
}
