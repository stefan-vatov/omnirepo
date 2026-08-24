//! Canonical test-taxonomy and constitutional traceability validation.
//!
//! The matrix is data. This module only checks its shape and references; it
//! never selects an owner decision or infers a missing product rule.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path},
};

use yaml_serde::{Mapping, Value};

pub const MATRIX_SCHEMA: &str = "omnirepo.traceability-matrix.v1";
const FIXTURE_SCHEMA: &str = "omnirepo.traceability-fixture.v1";
const EVIDENCE_SCHEMA: &str = "omnirepo.traceability-evidence.v1";
const MAX_FINDINGS: usize = 64;
const MAX_TEXT: usize = 256;
// These are validator input-safety bounds, not product synchronization
// limits. The first product release deliberately has no numeric content or
// storage policy; the validator must still avoid unbounded hostile input.
const MAX_MATRIX_BYTES: usize = 1_048_576;
const MAX_BEADS_BYTES: usize = 8 * 1_048_576;
const MAX_ROWS: usize = 1_024;
const MAX_NESTING_DEPTH: usize = 32;
const MAX_STRING_BYTES: usize = 4_096;
const MAX_BEAD_STRING_BYTES: usize = 65_536;
const MAX_RECORD_BYTES: usize = 64 * 1_024;

const CONSTITUTION_PATH: &str = "CONSTITUTION.md";
const TENSION_ONE_LIMITS: &[&str] = &["outside-machine-fleet", "managed-partial-delimiters"];

const REQUIRED_COMMANDS: &[&str] = &["command:sync", "command:setup", "command:doctor"];

const REQUIRED_FAILURE_STAGES: &[&str] = &[
    "failure:invocation",
    "failure:run-record-create",
    "failure:machine-configuration",
    "failure:source-acquisition",
    "failure:source-catalog",
    "failure:repository-admission",
    "failure:repository-policy",
    "failure:planning",
    "failure:synchronization",
    "failure:verification",
    "failure:repair",
    "failure:git-commit",
    "failure:git-push",
    "failure:finalization",
    "failure:cancellation-recovery",
];

const REQUIRED_BEHAVIORS: &[&str] = &[
    "behavior:configuration-authority",
    "behavior:source-materialization",
    "behavior:repository-policy",
    "behavior:whole-file-sync",
    "behavior:partial-section-sync",
    "behavior:containment",
    "behavior:fleet-progress",
    "behavior:verification",
    "behavior:git-delivery",
    "behavior:run-record",
    "behavior:repair-causation",
    "behavior:setup",
    "behavior:doctor",
    "behavior:packaging",
];

const TEST_TYPES: &[&str] = &[
    "unit",
    "component",
    "black-box-e2e",
    "adversarial",
    "platform",
    "scale",
    "optional",
];

const VIEWS: &[&str] = &[
    "unit",
    "component",
    "black-box-e2e",
    "adversarial",
    "platform",
    "scale",
    "optional",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub path: String,
    pub message: String,
    pub replay_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub valid: bool,
    pub rows: usize,
    pub findings: Vec<Finding>,
    pub truncated: bool,
    pub replay_id: String,
}

#[derive(Debug)]
pub struct ValidationError(String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

/// Validate the checked-in matrix and the repository's tracked Beads export.
pub fn validate_file(path: &Path) -> Result<Report, ValidationError> {
    let repository = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR"))).map_err(|error| {
        ValidationError(format!(
            "cannot resolve repository root {}: {error}",
            Path::new(env!("CARGO_MANIFEST_DIR")).display()
        ))
    })?;
    let beads_path = repository.join(".beads/issues.jsonl");
    validate_file_with_beads_path(path, &beads_path, &repository)
}

/// Validate a matrix fixture against an explicit, repository-contained Beads
/// export. This seam is test-only so file-backed proof tests can use isolated
/// structured records without changing the tracked project export.
#[cfg(test)]
pub fn validate_file_with_beads(path: &Path, beads_path: &Path) -> Result<Report, ValidationError> {
    let repository = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR"))).map_err(|error| {
        ValidationError(format!(
            "cannot resolve repository root {}: {error}",
            Path::new(env!("CARGO_MANIFEST_DIR")).display()
        ))
    })?;
    validate_file_with_beads_path(path, beads_path, &repository)
}

fn validate_file_with_beads_path(
    path: &Path,
    beads_path: &Path,
    repository: &Path,
) -> Result<Report, ValidationError> {
    let matrix = read_repository_file(path, repository, MAX_MATRIX_BYTES, "traceability matrix")?;
    let beads = read_repository_file(
        beads_path,
        repository,
        MAX_BEADS_BYTES,
        "tracked Beads export",
    )?;
    let constitution_path = repository.join(CONSTITUTION_PATH);
    let constitution = read_repository_file(
        &constitution_path,
        repository,
        MAX_MATRIX_BYTES,
        "constitution",
    )?;
    Ok(validate_source_with_constitution_at(
        &matrix,
        &beads,
        &constitution,
        Some(repository),
    ))
}

/// Validate source text. This pure entry point makes malformed and stale cases
/// table-driven without touching HOME, Git, the network, or the tracker.
pub fn validate_source(matrix_source: &str, beads_source: &str) -> Report {
    validate_source_with_constitution(
        matrix_source,
        beads_source,
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/CONSTITUTION.md")),
    )
}

/// Validate source text against an explicit Constitution snapshot.
pub fn validate_source_with_constitution(
    matrix_source: &str,
    beads_source: &str,
    constitution_source: &str,
) -> Report {
    validate_source_with_constitution_at(matrix_source, beads_source, constitution_source, None)
}

fn validate_source_with_constitution_at(
    matrix_source: &str,
    beads_source: &str,
    constitution_source: &str,
    repository: Option<&Path>,
) -> Report {
    let replay_id = "traceability-validator.v1".to_owned();
    let mut validator = Validator {
        findings: Vec::new(),
        truncated: false,
        replay_id: replay_id.clone(),
        repository,
    };
    if matrix_source.len() > MAX_MATRIX_BYTES {
        validator.push(
            "matrix-too-large",
            "$",
            format!("matrix exceeds {MAX_MATRIX_BYTES} bytes"),
        );
        return validator.finish(0);
    }
    if beads_source.len() > MAX_BEADS_BYTES {
        validator.push(
            "beads-too-large",
            ".beads/issues.jsonl",
            format!("tracked Beads export exceeds {MAX_BEADS_BYTES} bytes"),
        );
        return validator.finish(0);
    }
    if let Err(error) = StrictJson::new(matrix_source, MAX_NESTING_DEPTH, MAX_STRING_BYTES).parse()
    {
        validator.push(
            "schema-malformed",
            "$",
            format!("matrix is not strict JSON: {error}"),
        );
        return validator.finish(0);
    }
    let expected_clauses = match constitution_clause_ids(constitution_source) {
        Ok(clauses) => clauses,
        Err(error) => {
            validator.push("constitution-anchor-missing", CONSTITUTION_PATH, error);
            return validator.finish(0);
        }
    };
    let document = match yaml_serde::from_str::<Value>(matrix_source) {
        Ok(value) => value,
        Err(error) => {
            validator.push(
                "schema-malformed",
                "$",
                format!("strict JSON could not be decoded: {error}"),
            );
            return validator.finish(0);
        }
    };
    let Some(root) = document.as_mapping() else {
        validator.push("schema-malformed", "$", "matrix root must be an object");
        return validator.finish(0);
    };

    validator.reject_unknown_keys(
        root,
        "root",
        &[
            "schema",
            "status",
            "taxonomy",
            "required_clause_ids",
            "required_public_commands",
            "required_failure_stages",
            "required_behavior_ids",
            "constitution_source",
            "validator_limits",
            "rows",
        ],
    );
    validator.require_string(root, "schema", MATRIX_SCHEMA, "root.schema");
    validator.require_string(root, "status", "canonical", "root.status");
    validator.require_string(
        root,
        "constitution_source",
        CONSTITUTION_PATH,
        "root.constitution_source",
    );
    validator.validate_limits(root.get(Value::String("validator_limits".to_owned())));
    validator.validate_taxonomy(root.get(Value::String("taxonomy".to_owned())));
    validator.validate_id_list(
        root.get(Value::String("required_clause_ids".to_owned())),
        &expected_clauses,
        "required_clause_ids",
    );
    validator.validate_id_list(
        root.get(Value::String("required_public_commands".to_owned())),
        REQUIRED_COMMANDS,
        "required_public_commands",
    );
    validator.validate_id_list(
        root.get(Value::String("required_failure_stages".to_owned())),
        REQUIRED_FAILURE_STAGES,
        "required_failure_stages",
    );
    validator.validate_id_list(
        root.get(Value::String("required_behavior_ids".to_owned())),
        REQUIRED_BEHAVIORS,
        "required_behavior_ids",
    );

    let beads = collect_bead_records(beads_source, &mut validator);
    let mut expected = BTreeSet::new();
    expected.extend(expected_clauses.iter().map(String::as_str));
    expected.extend(REQUIRED_COMMANDS.iter().copied());
    expected.extend(REQUIRED_FAILURE_STAGES.iter().copied());
    expected.extend(REQUIRED_BEHAVIORS.iter().copied());

    let rows = root
        .get(Value::String("rows".to_owned()))
        .and_then(Value::as_sequence);
    let Some(rows) = rows else {
        validator.push("schema-missing", "rows", "rows must be an array");
        return validator.finish(0);
    };
    if rows.is_empty() {
        validator.push("schema-missing", "rows", "rows must not be empty");
    } else if rows.len() > MAX_ROWS {
        validator.push("rows-too-many", "rows", format!("rows exceed {MAX_ROWS}"));
    }
    let mut seen = SeenRows::default();
    for (index, row) in rows.iter().enumerate() {
        validator.validate_row(row, index, &beads, &expected, &mut seen);
    }
    for required in expected {
        if !seen.references.contains(required) {
            validator.push(
                "missing-required-row",
                "rows",
                format!("required traceability reference {required} has no primary row"),
            );
        }
    }
    validator.finish(rows.len())
}

fn read_repository_file(
    path: &Path,
    repository: &Path,
    limit: usize,
    label: &str,
) -> Result<String, ValidationError> {
    if !path.is_absolute() {
        return Err(ValidationError(format!(
            "{label} path must be absolute and repository-contained: {}",
            path.display()
        )));
    }
    let repository = fs::canonicalize(repository).map_err(|error| {
        ValidationError(format!(
            "cannot resolve repository root {}: {error}",
            repository.display()
        ))
    })?;
    let relative = path.strip_prefix(&repository).map_err(|_| {
        ValidationError(format!(
            "{label} path is outside the repository: {}",
            path.display()
        ))
    })?;
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(ValidationError(format!(
            "{label} path is not a contained relative path: {}",
            path.display()
        )));
    }
    let mut current = repository.clone();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(ValidationError(format!(
                "{label} path contains an unsupported component: {}",
                path.display()
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            ValidationError(format!(
                "cannot inspect {label} {}: {error}",
                current.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ValidationError(format!(
                "{label} path contains a symlink: {}",
                current.display()
            )));
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ValidationError(format!(
            "cannot inspect {label} {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() {
        return Err(ValidationError(format!(
            "{label} path is not a regular file: {}",
            path.display()
        )));
    }
    let mut file = fs::File::open(path).map_err(|error| {
        ValidationError(format!("cannot open {label} {}: {error}", path.display()))
    })?;
    let mut bytes = Vec::with_capacity(limit.min(8192));
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ValidationError(format!("cannot read {label} {}: {error}", path.display()))
        })?;
    if bytes.len() > limit {
        return Err(ValidationError(format!(
            "{label} exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| ValidationError(format!("{label} is not valid UTF-8: {}", path.display())))
}

fn constitution_clause_ids(source: &str) -> Result<Vec<String>, String> {
    let sections = [
        ("Founding Principles", "principle", 8usize),
        ("Growth Directives", "growth-directive", 5usize),
        ("Boundaries", "boundary", 5usize),
        ("Tension Pairs", "tension", 6usize),
    ];
    let mut output = Vec::new();
    for (heading, prefix, expected_count) in sections {
        let mut active = false;
        let mut ordinals = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed == format!("## {heading}") {
                active = true;
                continue;
            }
            if active && trimmed.starts_with("## ") {
                break;
            }
            if !active {
                continue;
            }
            let Some((ordinal, rest)) = trimmed.split_once('.') else {
                continue;
            };
            if !ordinal.bytes().all(|byte| byte.is_ascii_digit()) || !rest.starts_with(" **") {
                continue;
            }
            let number = ordinal
                .parse::<usize>()
                .map_err(|_| format!("{heading} contains an invalid ordinal {ordinal:?}"))?;
            if number == 0 || number > expected_count || ordinals.contains(&number) {
                return Err(format!(
                    "{heading} contains an invalid or duplicate ordinal {number}"
                ));
            }
            ordinals.push(number);
        }
        ordinals.sort_unstable();
        if ordinals != (1..=expected_count).collect::<Vec<_>>() {
            return Err(format!(
                "{heading} must expose ordinals 1..={expected_count} in CONSTITUTION.md"
            ));
        }
        output.extend((1..=expected_count).map(|number| format!("constitution:{prefix}.{number}")));
    }
    if !source.contains("outside the machine-declared fleet") {
        return Err("Tension Pair 1 lost the outside-machine-fleet limit".to_owned());
    }
    if !source.contains("allowing the sync engine itself to cross managed partial delimiters") {
        return Err("Tension Pair 1 lost the managed-partial-delimiters limit".to_owned());
    }
    Ok(output)
}

struct StrictJson<'a> {
    source: &'a str,
    bytes: &'a [u8],
    position: usize,
    max_depth: usize,
    max_string_bytes: usize,
}

impl<'a> StrictJson<'a> {
    fn new(source: &'a str, max_depth: usize, max_string_bytes: usize) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            position: 0,
            max_depth,
            max_string_bytes,
        }
    }

    fn parse(mut self) -> Result<(), String> {
        self.skip_whitespace();
        self.parse_value(0)?;
        self.skip_whitespace();
        if self.position != self.bytes.len() {
            return Err(self.error("trailing bytes after JSON value"));
        }
        Ok(())
    }

    fn parse_value(&mut self, depth: usize) -> Result<(), String> {
        if depth > self.max_depth {
            return Err(self.error("JSON nesting depth exceeds validator limit"));
        }
        self.skip_whitespace();
        match self.bytes.get(self.position).copied() {
            Some(b'{') => self.parse_object(depth + 1),
            Some(b'[') => self.parse_array(depth + 1),
            Some(b'"') => self.parse_string().map(|_| ()),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(b't') => self.parse_literal(b"true"),
            Some(b'f') => self.parse_literal(b"false"),
            Some(b'n') => self.parse_literal(b"null"),
            Some(_) => Err(self.error("unexpected JSON token")),
            None => Err(self.error("unexpected end of JSON input")),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<(), String> {
        self.position += 1;
        self.skip_whitespace();
        let mut keys = BTreeSet::new();
        if self.consume(b'}') {
            return Ok(());
        }
        loop {
            self.skip_whitespace();
            if self.bytes.get(self.position) != Some(&b'"') {
                return Err(self.error("JSON object key must be a string"));
            }
            let key = self.parse_string()?;
            if !keys.insert(key) {
                return Err(self.error("duplicate JSON object key"));
            }
            self.skip_whitespace();
            if !self.consume(b':') {
                return Err(self.error("JSON object key must be followed by a colon"));
            }
            self.parse_value(depth)?;
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(self.error("JSON object entries require a comma"));
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<(), String> {
        self.position += 1;
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(());
        }
        loop {
            self.parse_value(depth)?;
            self.skip_whitespace();
            if self.consume(b']') {
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(self.error("JSON array entries require a comma"));
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if !self.consume(b'"') {
            return Err(self.error("JSON string must start with a quote"));
        }
        let mut output = String::new();
        loop {
            let byte = *self
                .bytes
                .get(self.position)
                .ok_or_else(|| self.error("unterminated JSON string"))?;
            match byte {
                b'"' => {
                    self.position += 1;
                    return Ok(output);
                }
                b'\\' => {
                    self.position += 1;
                    let escape = *self
                        .bytes
                        .get(self.position)
                        .ok_or_else(|| self.error("unterminated JSON escape"))?;
                    self.position += 1;
                    let character = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.parse_unicode_escape()?,
                        _ => return Err(self.error("unsupported JSON string escape")),
                    };
                    output.push(character);
                }
                byte if byte < 0x20 => {
                    return Err(self.error("JSON strings cannot contain control bytes"));
                }
                _ => {
                    let character = self.source[self.position..]
                        .chars()
                        .next()
                        .ok_or_else(|| self.error("invalid UTF-8 in JSON string"))?;
                    self.position += character.len_utf8();
                    output.push(character);
                }
            }
            if output.len() > self.max_string_bytes {
                return Err(self.error("JSON string exceeds validator limit"));
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let value = self.parse_unicode_code_unit()?;
        if (0xD800..=0xDBFF).contains(&value) {
            if !self.consume(b'\\') || !self.consume(b'u') {
                return Err(self.error("high JSON surrogate must have a low surrogate"));
            }
            let low = self.parse_unicode_code_unit()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(self.error("high JSON surrogate has an invalid low surrogate"));
            }
            let scalar = 0x1_0000 + ((value - 0xD800) << 10) + (low - 0xDC00);
            return char::from_u32(scalar)
                .ok_or_else(|| self.error("invalid JSON Unicode surrogate pair"));
        }
        if (0xDC00..=0xDFFF).contains(&value) {
            return Err(self.error("low JSON surrogate has no high surrogate"));
        }
        char::from_u32(value).ok_or_else(|| self.error("invalid JSON Unicode scalar"))
    }

    fn parse_unicode_code_unit(&mut self) -> Result<u32, String> {
        if self.position + 4 > self.bytes.len() {
            return Err(self.error("short JSON Unicode escape"));
        }
        let digits = &self.bytes[self.position..self.position + 4];
        self.position += 4;
        let mut value = 0u32;
        for digit in digits {
            value = value * 16
                + match digit {
                    b'0'..=b'9' => u32::from(digit - b'0'),
                    b'a'..=b'f' => u32::from(digit - b'a' + 10),
                    b'A'..=b'F' => u32::from(digit - b'A' + 10),
                    _ => return Err(self.error("invalid JSON Unicode escape")),
                };
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<(), String> {
        let start = self.position;
        self.consume(b'-');
        match self.bytes.get(self.position).copied() {
            Some(b'0') => {
                self.position += 1;
                if self
                    .bytes
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    return Err(self.error("JSON numbers cannot have leading zeroes"));
                }
            }
            Some(b'1'..=b'9') => {
                self.position += 1;
                while self
                    .bytes
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    self.position += 1;
                }
            }
            _ => return Err(self.error("JSON number requires an integer")),
        }
        if self.consume(b'.') {
            let fraction_start = self.position;
            while self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if self.position == fraction_start {
                return Err(self.error("JSON fraction requires digits"));
            }
        }
        if self
            .bytes
            .get(self.position)
            .is_some_and(|byte| *byte == b'e' || *byte == b'E')
        {
            self.position += 1;
            self.consume(b'+');
            self.consume(b'-');
            let exponent_start = self.position;
            while self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if self.position == exponent_start {
                return Err(self.error("JSON exponent requires digits"));
            }
        }
        if start == self.position {
            return Err(self.error("empty JSON number"));
        }
        Ok(())
    }

    fn parse_literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.bytes.get(self.position..self.position + literal.len()) == Some(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(self.error("invalid JSON literal"))
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.position += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.position) == Some(&expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.position)
    }
}

#[derive(Debug, Clone)]
struct BeadRecord {
    status: String,
    issue_type: String,
    labels: BTreeSet<String>,
    has_close_provenance: bool,
    traceability_evidence: Vec<TraceabilityEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceabilityEvidence {
    schema: String,
    row_id: String,
    case_id: String,
    evidence_id: String,
    locator_role: String,
    downstream_bead: String,
}

#[derive(Default)]
struct SeenRows {
    ids: BTreeSet<String>,
    references: BTreeSet<String>,
    cases: BTreeSet<String>,
    evidence: BTreeSet<String>,
    replays: BTreeSet<String>,
    fixtures: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy)]
struct LocatorBinding<'a> {
    row_id: &'a str,
    case_id: &'a str,
    identity: &'a str,
    downstream_bead: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct StatusBinding<'a> {
    row_id: &'a str,
    case_id: &'a str,
    evidence_id: &'a str,
    downstream_bead: &'a str,
}

struct Validator<'a> {
    findings: Vec<Finding>,
    truncated: bool,
    replay_id: String,
    repository: Option<&'a Path>,
}

impl Validator<'_> {
    fn finish(self, rows: usize) -> Report {
        Report {
            valid: self.findings.is_empty(),
            rows,
            findings: self.findings,
            truncated: self.truncated,
            replay_id: self.replay_id,
        }
    }

    fn push(&mut self, code: &str, path: impl Into<String>, message: impl Into<String>) {
        if self.findings.len() >= MAX_FINDINGS {
            self.truncated = true;
            return;
        }
        let path = bounded(path.into());
        let message = bounded(message.into());
        let replay_id = format!(
            "traceability/{code}/{}",
            stable_digest(&format!("{code}\n{path}\n{message}"))
        );
        self.findings.push(Finding {
            code: code.to_owned(),
            path,
            message,
            replay_id,
        });
    }

    fn reject_unknown_keys(&mut self, mapping: &Mapping, path: &str, allowed: &[&str]) {
        for (key, _) in mapping {
            let Some(key) = key.as_str() else {
                self.push("schema-malformed", path, "object keys must be strings");
                continue;
            };
            if !allowed.contains(&key) {
                self.push(
                    "schema-unknown-field",
                    format!("{path}.{key}"),
                    "field is not part of matrix schema",
                );
            }
            if is_policy_selecting_key(key) {
                self.push(
                    "policy-value-forbidden",
                    format!("{path}.{key}"),
                    "matrix may reference owner decisions but may not select their value",
                );
            }
        }
    }

    fn require_string(&mut self, mapping: &Mapping, key: &str, expected: &str, path: &str) {
        match mapping
            .get(Value::String(key.to_owned()))
            .and_then(Value::as_str)
        {
            Some(actual) if actual == expected => {}
            Some(actual) => self.push(
                "schema-value",
                path,
                format!("expected {expected:?}, got {actual:?}"),
            ),
            None => self.push(
                "schema-missing",
                path,
                format!("required string is {expected:?}"),
            ),
        }
    }

    fn validate_taxonomy(&mut self, value: Option<&Value>) {
        let Some(mapping) = value.and_then(Value::as_mapping) else {
            self.push("schema-missing", "taxonomy", "taxonomy must be an object");
            return;
        };
        self.reject_unknown_keys(
            mapping,
            "taxonomy",
            &["test_types", "effect_kinds", "views"],
        );
        self.validate_enum_list(
            mapping.get(Value::String("test_types".to_owned())),
            TEST_TYPES,
            "taxonomy.test_types",
        );
        self.validate_enum_list(
            mapping.get(Value::String("views".to_owned())),
            VIEWS,
            "taxonomy.views",
        );
        self.validate_enum_list(
            mapping.get(Value::String("effect_kinds".to_owned())),
            &["positive", "negative", "conditional", "silence"],
            "taxonomy.effect_kinds",
        );
    }

    fn validate_limits(&mut self, value: Option<&Value>) {
        let Some(mapping) = value.and_then(Value::as_mapping) else {
            self.push(
                "schema-missing",
                "validator_limits",
                "validator_limits must be an object",
            );
            return;
        };
        self.reject_unknown_keys(
            mapping,
            "validator_limits",
            &[
                "matrix_bytes",
                "beads_bytes",
                "bead_string_bytes",
                "rows",
                "nesting_depth",
                "string_bytes",
            ],
        );
        for (key, expected) in [
            ("matrix_bytes", MAX_MATRIX_BYTES),
            ("beads_bytes", MAX_BEADS_BYTES),
            ("bead_string_bytes", MAX_BEAD_STRING_BYTES),
            ("rows", MAX_ROWS),
            ("nesting_depth", MAX_NESTING_DEPTH),
            ("string_bytes", MAX_STRING_BYTES),
        ] {
            let actual = mapping
                .get(Value::String(key.to_owned()))
                .and_then(Value::as_u64);
            if actual != Some(expected as u64) {
                self.push(
                    "schema-value",
                    format!("validator_limits.{key}"),
                    format!("expected validator bound {expected}, got {actual:?}"),
                );
            }
        }
    }

    fn validate_enum_list(&mut self, value: Option<&Value>, required: &[&str], path: &str) {
        let Some(sequence) = value.and_then(Value::as_sequence) else {
            self.push(
                "schema-missing",
                path,
                "required enum list is missing or not an array",
            );
            return;
        };
        let actual = sequence
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        let required = required.iter().copied().collect::<BTreeSet<_>>();
        if actual != required {
            for missing in required.difference(&actual) {
                self.push(
                    "schema-missing",
                    path,
                    format!("required enum {missing:?} is missing"),
                );
            }
            for extra in actual.difference(&required) {
                self.push(
                    "schema-unexpected",
                    path,
                    format!("enum {extra:?} is not supported"),
                );
            }
        }
        if sequence.len() != actual.len() {
            self.push(
                "schema-duplicate",
                path,
                "enum list contains duplicate or non-string entries",
            );
        }
    }

    fn validate_id_list<T: AsRef<str>>(
        &mut self,
        value: Option<&Value>,
        required: &[T],
        path: &str,
    ) {
        let Some(sequence) = value.and_then(Value::as_sequence) else {
            self.push(
                "schema-missing",
                path,
                "required identifier list is missing or not an array",
            );
            return;
        };
        let mut actual = BTreeSet::new();
        for (index, item) in sequence.iter().enumerate() {
            let Some(item) = item.as_str() else {
                self.push(
                    "schema-malformed",
                    format!("{path}[{index}]"),
                    "identifier must be a string",
                );
                continue;
            };
            if !actual.insert(item) {
                self.push(
                    "schema-duplicate",
                    format!("{path}[{index}]"),
                    format!("duplicate identifier {item}"),
                );
            }
        }
        let required = required.iter().map(AsRef::as_ref).collect::<BTreeSet<_>>();
        if actual != required {
            for missing in required.difference(&actual) {
                self.push(
                    "schema-missing",
                    path,
                    format!("required identifier {missing} is missing"),
                );
            }
            for extra in actual.difference(&required) {
                self.push(
                    "schema-unexpected",
                    path,
                    format!("identifier {extra} is not required by this schema"),
                );
            }
        }
    }

    fn validate_row(
        &mut self,
        value: &Value,
        index: usize,
        beads: &BTreeMap<String, BeadRecord>,
        required: &BTreeSet<&str>,
        seen: &mut SeenRows,
    ) {
        let Some(row) = value.as_mapping() else {
            self.push(
                "schema-malformed",
                format!("rows[{index}]"),
                "row must be an object",
            );
            return;
        };
        // Row diagnostics use the stable row identity when it is available.
        // This keeps replay IDs unchanged when an unrelated row is inserted.
        let path = row
            .get(Value::String("id".to_owned()))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map_or_else(|| format!("rows[{index}]"), |id| format!("rows[id={id}]"));
        self.reject_unknown_keys(
            row,
            &path,
            &[
                "id",
                "kind",
                "reference",
                "coverage_status",
                "test_type",
                "primary_owner",
                "implementation_bead",
                "implementation_status",
                "verification_status",
                "test_locator",
                "evidence_locator",
                "supporting_views",
                "fixture",
                "fixture_locator",
                "limits",
                "expected_effect",
                "expected_observation",
                "negative_case",
                "case_id",
                "evidence_id",
                "replay_id",
                "downstream_bead",
                "owner_decision_refs",
                "constitutional_silence",
            ],
        );

        let id = self.required_stable_string(row, "id", &path, "stable row ID");
        if let Some(id) = id.as_deref()
            && !seen.ids.insert(id.to_owned())
        {
            self.push(
                "duplicate-row-id",
                format!("{path}.id"),
                format!("duplicate row ID {id}"),
            );
        }
        let reference = self.required_string(row, "reference", &path, "required reference");
        if let Some(reference) = reference.as_deref() {
            if !seen.references.insert(reference.to_owned()) {
                self.push(
                    "duplicate-primary-owner",
                    format!("{path}.reference"),
                    format!("reference {reference} has more than one primary test owner"),
                );
            }
            if !required.contains(reference) && !reference.starts_with("optional:") {
                self.push(
                    "orphan-reference",
                    format!("{path}.reference"),
                    format!("reference {reference} is not in the required taxonomy"),
                );
            }
        }
        self.require_enum(
            row,
            "kind",
            &[
                "constitutional",
                "public-command",
                "failure-stage",
                "product-contract",
                "optional",
            ],
            &path,
        );
        let coverage = self.require_enum(
            row,
            "coverage_status",
            &["required", "conditional", "optional", "silence"],
            &path,
        );
        let test_type = self.require_enum(row, "test_type", TEST_TYPES, &path);
        if let Some(reference) = reference.as_deref() {
            if required.contains(reference)
                && !matches!(coverage.as_deref(), Some("required" | "conditional"))
            {
                self.push(
                    "required-row-not-required",
                    format!("{path}.coverage_status"),
                    "a required taxonomy reference cannot be marked optional or silent",
                );
            }
            let expected_kind = if reference.starts_with("constitution:") {
                "constitutional"
            } else if reference.starts_with("command:") {
                "public-command"
            } else if reference.starts_with("failure:") {
                "failure-stage"
            } else if reference.starts_with("behavior:") {
                "product-contract"
            } else {
                "optional"
            };
            if row
                .get(Value::String("kind".to_owned()))
                .and_then(Value::as_str)
                != Some(expected_kind)
            {
                self.push(
                    "schema-value",
                    format!("{path}.kind"),
                    format!("reference prefix requires kind {expected_kind:?}"),
                );
            }
        }
        self.require_work_bead(row, "primary_owner", &path, beads);
        self.require_work_bead(row, "implementation_bead", &path, beads);
        let implementation_status = self.require_enum(
            row,
            "implementation_status",
            &["specified", "implemented"],
            &path,
        );
        let verification_status = self.require_enum(
            row,
            "verification_status",
            &["specified", "verified"],
            &path,
        );
        let implementation_bead = row
            .get(Value::String("implementation_bead".to_owned()))
            .and_then(Value::as_str)
            .unwrap_or("");
        let downstream_bead = row
            .get(Value::String("downstream_bead".to_owned()))
            .and_then(Value::as_str)
            .unwrap_or("");
        let row_id_for_contract = row
            .get(Value::String("id".to_owned()))
            .and_then(Value::as_str)
            .unwrap_or("");
        let case_id_for_contract = row
            .get(Value::String("case_id".to_owned()))
            .and_then(Value::as_str)
            .unwrap_or("");
        let evidence_id_for_contract = row
            .get(Value::String("evidence_id".to_owned()))
            .and_then(Value::as_str)
            .unwrap_or("");
        self.validate_status_pair(
            row,
            &path,
            beads,
            implementation_status.as_deref(),
            verification_status.as_deref(),
            StatusBinding {
                row_id: row_id_for_contract,
                case_id: case_id_for_contract,
                evidence_id: evidence_id_for_contract,
                downstream_bead,
            },
        );
        let test_role = if implementation_status.as_deref() == Some("implemented") {
            "executable"
        } else {
            "planned"
        };
        let evidence_role = if verification_status.as_deref() == Some("verified") {
            "artifact"
        } else {
            "planned"
        };
        self.validate_locator(
            row.get(Value::String("test_locator".to_owned())),
            &path,
            "test_locator",
            test_role,
            Some(&format!("{implementation_bead}#{case_id_for_contract}")),
            None,
        );
        self.validate_locator(
            row.get(Value::String("evidence_locator".to_owned())),
            &path,
            "evidence_locator",
            evidence_role,
            Some(&format!("{downstream_bead}#{evidence_id_for_contract}")),
            Some(LocatorBinding {
                row_id: row_id_for_contract,
                case_id: case_id_for_contract,
                identity: evidence_id_for_contract,
                downstream_bead,
            }),
        );
        let fixture = self.required_identity_string(row, "fixture", &path, "fixture identity");
        if let Some(fixture) = fixture.as_deref()
            && !seen.fixtures.insert(fixture.to_owned())
        {
            self.push(
                "duplicate-fixture",
                format!("{path}.fixture"),
                format!("fixture identity {fixture} is used by more than one row"),
            );
        }
        if implementation_status.as_deref() == Some("implemented") {
            self.validate_locator(
                row.get(Value::String("fixture_locator".to_owned())),
                &path,
                "fixture_locator",
                "fixture",
                fixture.as_deref(),
                Some(LocatorBinding {
                    row_id: row_id_for_contract,
                    case_id: case_id_for_contract,
                    identity: fixture.as_deref().unwrap_or(""),
                    downstream_bead,
                }),
            );
        } else if row
            .get(Value::String("fixture_locator".to_owned()))
            .is_some()
        {
            self.push(
                "locator-role-mismatch",
                format!("{path}.fixture_locator"),
                "specified rows may not claim an executable fixture locator",
            );
        }
        self.validate_limits_for_row(
            row.get(Value::String("limits".to_owned())),
            &path,
            reference.as_deref(),
        );
        self.require_work_bead(row, "downstream_bead", &path, beads);
        self.require_nonempty(row, "expected_observation", &path);
        self.require_nonempty(row, "negative_case", &path);
        let case_id = self.required_stable_string(row, "case_id", &path, "stable case ID");
        if let Some(case_id) = case_id
            && !seen.cases.insert(case_id.clone())
        {
            self.push(
                "duplicate-case-id",
                format!("{path}.case_id"),
                format!("case identity {case_id} is used by more than one row"),
            );
        }
        let evidence_id =
            self.required_stable_string(row, "evidence_id", &path, "stable evidence ID");
        if let Some(evidence_id) = evidence_id
            && !seen.evidence.insert(evidence_id.clone())
        {
            self.push(
                "duplicate-evidence-id",
                format!("{path}.evidence_id"),
                format!("evidence identity {evidence_id} is used by more than one row"),
            );
        }
        let replay_id =
            self.required_stable_string(row, "replay_id", &path, "stable replay identity");
        if let Some(replay_id) = replay_id
            && !seen.replays.insert(replay_id.clone())
        {
            self.push(
                "duplicate-replay-id",
                format!("{path}.replay_id"),
                format!("replay identity {replay_id} is used by more than one row"),
            );
        }
        let expected_effect = self.require_enum(
            row,
            "expected_effect",
            &["positive", "negative", "conditional", "silence"],
            &path,
        );
        let silence = match row.get(Value::String("constitutional_silence".to_owned())) {
            Some(Value::Bool(value)) => Some(*value),
            Some(_) => {
                self.push(
                    "silence-not-explicit",
                    format!("{path}.constitutional_silence"),
                    "constitutional_silence must be an explicit boolean",
                );
                None
            }
            None => {
                self.push(
                    "silence-not-explicit",
                    format!("{path}.constitutional_silence"),
                    "every row must declare constitutional_silence as true or false",
                );
                None
            }
        };
        self.validate_silence(
            &path,
            coverage.as_deref(),
            test_type.as_deref(),
            expected_effect.as_deref(),
            silence,
        );
        self.validate_views(row.get(Value::String("supporting_views".to_owned())), &path);
        self.validate_projections(
            row,
            &path,
            reference.as_deref(),
            test_type.as_deref(),
            expected_effect.as_deref(),
        );
        self.validate_bead_refs(
            row.get(Value::String("owner_decision_refs".to_owned())),
            &path,
            beads,
        );
        self.reject_policy_values(value, &path);
    }

    fn required_string(
        &mut self,
        mapping: &Mapping,
        key: &str,
        path: &str,
        description: &str,
    ) -> Option<String> {
        match mapping
            .get(Value::String(key.to_owned()))
            .and_then(Value::as_str)
        {
            Some(value) if !value.trim().is_empty() => Some(value.to_owned()),
            _ => {
                self.push("schema-missing", format!("{path}.{key}"), description);
                None
            }
        }
    }

    fn required_stable_string(
        &mut self,
        mapping: &Mapping,
        key: &str,
        path: &str,
        description: &str,
    ) -> Option<String> {
        let value = self.required_string(mapping, key, path, description);
        if let Some(value) = value.as_deref()
            && !is_stable_id(value)
        {
            self.push(
                    "schema-value",
                    format!("{path}.{key}"),
                    "identifier must use lowercase ASCII letters, digits, dots, underscores, or hyphens",
                );
        }
        value
    }

    fn require_nonempty(&mut self, mapping: &Mapping, key: &str, path: &str) {
        let _ = self.required_string(mapping, key, path, "required non-empty string");
    }

    fn required_identity_string(
        &mut self,
        mapping: &Mapping,
        key: &str,
        path: &str,
        description: &str,
    ) -> Option<String> {
        let value = self.required_string(mapping, key, path, description);
        if let Some(value) = value.as_deref()
            && !is_identity(value)
        {
            self.push(
                "schema-value",
                format!("{path}.{key}"),
                "identity must use lowercase ASCII namespace and stable characters",
            );
        }
        value
    }

    fn require_enum(
        &mut self,
        mapping: &Mapping,
        key: &str,
        values: &[&str],
        path: &str,
    ) -> Option<String> {
        let value = self.required_string(mapping, key, path, "required enum value")?;
        if !values.contains(&value.as_str()) {
            self.push(
                "schema-value",
                format!("{path}.{key}"),
                format!("unsupported enum value {value:?}"),
            );
        }
        Some(value)
    }

    fn require_work_bead(
        &mut self,
        mapping: &Mapping,
        key: &str,
        path: &str,
        beads: &BTreeMap<String, BeadRecord>,
    ) {
        let Some(value) = self.required_string(mapping, key, path, "required Bead ID") else {
            return;
        };
        let Some(record) = beads.get(&value) else {
            self.push(
                "orphan-bead",
                format!("{path}.{key}"),
                format!("Bead ID {value} is not present in the tracked export"),
            );
            return;
        };
        if record.issue_type == "decision"
            || record.status == "decision"
            || record.labels.contains("decision-needed")
            || record.labels.contains("human-input")
        {
            self.push(
                "owner-bead-not-work",
                format!("{path}.{key}"),
                format!(
                    "Bead ID {value} is an owner-decision record, not an implementation/work owner"
                ),
            );
        }
        if !matches!(
            record.issue_type.as_str(),
            "task" | "feature" | "bug" | "epic"
        ) {
            self.push(
                "owner-bead-not-work",
                format!("{path}.{key}"),
                format!(
                    "Bead ID {value} has unsupported work type {:?}",
                    record.issue_type
                ),
            );
        }
    }

    fn validate_status_pair(
        &mut self,
        mapping: &Mapping,
        path: &str,
        beads: &BTreeMap<String, BeadRecord>,
        implementation_status: Option<&str>,
        verification_status: Option<&str>,
        binding: StatusBinding<'_>,
    ) {
        let Some(implementation_bead) = mapping
            .get(Value::String("implementation_bead".to_owned()))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(record) = beads.get(implementation_bead) else {
            return;
        };
        if implementation_status == Some("implemented") && record.status != "closed" {
            self.push(
                "implementation-status-overclaim",
                format!("{path}.implementation_status"),
                "implemented status requires a closed implementation Bead",
            );
        }
        if verification_status == Some("verified") && implementation_status != Some("implemented") {
            self.push(
                "verification-status-overclaim",
                format!("{path}.verification_status"),
                "verified status requires implementation_status=implemented",
            );
        }
        if verification_status == Some("verified") {
            if implementation_bead == binding.downstream_bead && !implementation_bead.is_empty() {
                self.push(
                    "verification-downstream-same",
                    format!("{path}.downstream_bead"),
                    "verified status requires a distinct downstream acceptance Bead",
                );
            }
            let downstream = beads.get(binding.downstream_bead);
            let binding = TraceabilityEvidence {
                schema: EVIDENCE_SCHEMA.to_owned(),
                row_id: binding.row_id.to_owned(),
                case_id: binding.case_id.to_owned(),
                evidence_id: binding.evidence_id.to_owned(),
                locator_role: "artifact".to_owned(),
                downstream_bead: binding.downstream_bead.to_owned(),
            };
            match downstream {
                Some(record)
                    if record.status == "closed"
                        && record.has_close_provenance
                        && record
                            .traceability_evidence
                            .iter()
                            .any(|candidate| candidate == &binding) => {}
                Some(record) if record.status == "closed" => self.push(
                    "verification-evidence-provenance-missing",
                    format!("{path}.downstream_bead"),
                    "verified status requires exact structured evidence provenance in the closed downstream Bead",
                ),
                Some(_) => self.push(
                    "verification-downstream-not-accepted",
                    format!("{path}.downstream_bead"),
                    "verified status requires a closed downstream acceptance Bead",
                ),
                None => self.push(
                    "verification-downstream-not-accepted",
                    format!("{path}.downstream_bead"),
                    "verified status requires a tracked downstream acceptance Bead",
                ),
            }
        }
    }

    fn validate_locator(
        &mut self,
        value: Option<&Value>,
        path: &str,
        key: &str,
        expected_role: &str,
        expected_contract: Option<&str>,
        binding: Option<LocatorBinding<'_>>,
    ) {
        let locator_path = format!("{path}.{key}");
        let Some(mapping) = value.and_then(Value::as_mapping) else {
            self.push(
                "schema-missing",
                locator_path,
                "locator must contain a planned contract or executable path and selector",
            );
            return;
        };
        let role = self.required_string(mapping, "role", &format!("{path}.{key}"), "locator role");
        if role.as_deref() != Some(expected_role) {
            self.push(
                "locator-role-mismatch",
                format!("{path}.{key}.role"),
                format!("expected locator role {expected_role:?}"),
            );
        }
        if expected_role == "planned" {
            self.reject_unknown_keys(mapping, &format!("{path}.{key}"), &["role", "contract"]);
            let contract = self.required_string(
                mapping,
                "contract",
                &format!("{path}.{key}"),
                "planned locator contract",
            );
            if !contract.as_deref().is_some_and(is_contract_identity) {
                self.push(
                    "locator-contract-mismatch",
                    format!("{path}.{key}.contract"),
                    "planned locator contract must be a stable Bead#case/evidence identity",
                );
            }
            if let (Some(actual), Some(expected)) = (contract.as_deref(), expected_contract)
                && actual != expected
            {
                self.push(
                    "locator-contract-mismatch",
                    format!("{path}.{key}.contract"),
                    format!("planned locator must bind to {expected:?}"),
                );
            }
            return;
        }

        self.reject_unknown_keys(
            mapping,
            &format!("{path}.{key}"),
            &["role", "path", "selector"],
        );
        let Some(file_path) =
            self.required_string(mapping, "path", &format!("{path}.{key}"), "locator path")
        else {
            return;
        };
        let Some(selector) = self.required_string(
            mapping,
            "selector",
            &format!("{path}.{key}"),
            "locator selector",
        ) else {
            return;
        };
        if file_path.starts_with('/')
            || file_path
                .split('/')
                .any(|component| component == ".." || component.is_empty())
        {
            self.push(
                "locator-outside-repository",
                format!("{path}.{key}.path"),
                "locator path must be a non-empty repository-relative path",
            );
            return;
        }
        if !is_selector(&selector) {
            self.push(
                "schema-value",
                format!("{path}.{key}.selector"),
                "locator selector must use stable selector characters",
            );
        }
        if let Some(repository) = self.repository {
            self.resolve_locator(
                repository,
                expected_role,
                &file_path,
                &selector,
                &locator_path,
                binding,
            );
        }
    }

    fn resolve_locator(
        &mut self,
        repository: &Path,
        role: &str,
        file_path: &str,
        selector: &str,
        path: &str,
        binding: Option<LocatorBinding<'_>>,
    ) {
        let relative = Path::new(file_path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            self.push(
                "locator-outside-repository",
                format!("{path}.path"),
                "locator path must remain repository-contained",
            );
            return;
        }
        let mut current = repository.to_path_buf();
        for component in relative.components() {
            let Component::Normal(component) = component else {
                self.push(
                    "locator-outside-repository",
                    format!("{path}.path"),
                    "locator path has an unsupported component",
                );
                return;
            };
            current.push(component);
            let metadata = match fs::symlink_metadata(&current) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.push(
                        "locator-unresolved",
                        format!("{path}.path"),
                        format!("cannot inspect executable locator: {error}"),
                    );
                    return;
                }
            };
            if metadata.file_type().is_symlink() {
                self.push(
                    "locator-outside-repository",
                    format!("{path}.path"),
                    "executable locator may not traverse a symlink",
                );
                return;
            }
        }
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            self.push(
                "locator-unresolved",
                format!("{path}.path"),
                "executable locator file does not exist",
            );
            return;
        };
        if !metadata.file_type().is_file() {
            self.push(
                "locator-unresolved",
                format!("{path}.path"),
                "executable locator must name a regular file",
            );
            return;
        }
        let source = match read_repository_file(
            &current,
            repository,
            MAX_RECORD_BYTES,
            "executable locator",
        ) {
            Ok(source) => source,
            Err(error) => {
                self.push(
                    "locator-unresolved",
                    format!("{path}.selector"),
                    error.to_string(),
                );
                return;
            }
        };
        let result = match role {
            "executable" => resolve_rust_selector(&source, selector)
                .map_err(|error| format!("Rust selector is invalid or unresolved: {error}")),
            "fixture" | "artifact" => {
                let Some(binding) = binding else {
                    return self.push(
                        "locator-artifact-invalid",
                        format!("{path}.selector"),
                        "structured fixture or evidence locator is missing its identity binding",
                    );
                };
                parse_structured_locator_record(&source, role, selector, binding)
            }
            _ => Err(format!("unsupported locator role {role:?}")),
        };
        if let Err(error) = result {
            let code = if matches!(role, "fixture" | "artifact") {
                "locator-artifact-invalid"
            } else {
                "locator-unresolved"
            };
            self.push(
                code,
                format!("{path}.selector"),
                format!("selector {selector:?} does not resolve in {file_path}: {error}"),
            );
        }
    }

    fn validate_limits_for_row(
        &mut self,
        value: Option<&Value>,
        path: &str,
        reference: Option<&str>,
    ) {
        let Some(sequence) = value.and_then(Value::as_sequence) else {
            self.push(
                "schema-missing",
                format!("{path}.limits"),
                "limits must be an array",
            );
            return;
        };
        let mut actual = BTreeSet::new();
        for (index, item) in sequence.iter().enumerate() {
            let Some(limit) = item.as_str() else {
                self.push(
                    "schema-malformed",
                    format!("{path}.limits[{index}]"),
                    "limit identity must be a string",
                );
                continue;
            };
            if !is_stable_id(limit) {
                self.push(
                    "schema-value",
                    format!("{path}.limits[{index}]"),
                    "limit identity must use stable lowercase characters",
                );
            }
            if !TENSION_ONE_LIMITS.contains(&limit) {
                self.push(
                    "schema-unexpected",
                    format!("{path}.limits[{index}]"),
                    format!("unsupported constitutional limit identity {limit:?}"),
                );
            }
            if !actual.insert(limit) {
                self.push(
                    "schema-duplicate",
                    format!("{path}.limits[{index}]"),
                    format!("duplicate limit identity {limit}"),
                );
            }
        }
        if reference == Some("constitution:tension.1") {
            let required = TENSION_ONE_LIMITS.iter().copied().collect::<BTreeSet<_>>();
            if actual != required {
                self.push(
                    "missing-required-limit",
                    format!("{path}.limits"),
                    "Tension Pair 1 must name both outside-machine-fleet and managed-partial-delimiters limits",
                );
            }
        } else if !actual.is_empty()
            && reference.is_some_and(|value| !value.starts_with("constitution:tension."))
        {
            self.push(
                "schema-value",
                format!("{path}.limits"),
                "only tension-pair rows may declare constitutional limit identities",
            );
        }
    }

    fn validate_silence(
        &mut self,
        path: &str,
        coverage: Option<&str>,
        test_type: Option<&str>,
        expected_effect: Option<&str>,
        silence: Option<bool>,
    ) {
        let optional_coverage = matches!(coverage, Some("optional" | "silence"));
        let optional_type = test_type == Some("optional");
        let silent_effect = expected_effect == Some("silence");
        if optional_coverage {
            if silence != Some(true) || !silent_effect || !optional_type {
                self.push(
                    "silence-not-explicit",
                    path,
                    "optional or silent rows require optional test type, constitutional silence, and silence effect",
                );
            }
        } else if silence == Some(true) || silent_effect || optional_type {
            self.push(
                "silence-not-explicit",
                path,
                "required and conditional rows cannot silently opt out or use the optional test type",
            );
        }
    }

    fn validate_projections(
        &mut self,
        row: &Mapping,
        path: &str,
        reference: Option<&str>,
        test_type: Option<&str>,
        expected_effect: Option<&str>,
    ) {
        let kind = row
            .get(Value::String("kind".to_owned()))
            .and_then(Value::as_str);
        let constitutional_reference =
            reference.is_some_and(|value| value.starts_with("constitution:"));
        if constitutional_reference != (kind == Some("constitutional")) {
            self.push(
                "projection-mismatch",
                format!("{path}.kind"),
                "constitutional projection must be mechanically derived from constitutional references",
            );
        }

        let supports_adversarial = row
            .get(Value::String("supporting_views".to_owned()))
            .and_then(Value::as_sequence)
            .is_some_and(|views| {
                views
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|view| view == "adversarial")
            });
        let adversarial_projection = supports_adversarial
            || test_type == Some("adversarial")
            || expected_effect == Some("negative");
        if adversarial_projection
            && row
                .get(Value::String("negative_case".to_owned()))
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
        {
            self.push(
                "projection-missing-negative-case",
                format!("{path}.negative_case"),
                "adversarial projection requires an explicit negative case",
            );
        }
    }

    fn validate_views(&mut self, value: Option<&Value>, path: &str) {
        let Some(sequence) = value.and_then(Value::as_sequence) else {
            self.push(
                "schema-missing",
                format!("{path}.supporting_views"),
                "supporting_views must be an array",
            );
            return;
        };
        let mut seen = BTreeSet::new();
        for (index, item) in sequence.iter().enumerate() {
            let Some(view) = item.as_str() else {
                self.push(
                    "schema-malformed",
                    format!("{path}.supporting_views[{index}]"),
                    "view must be a string",
                );
                continue;
            };
            if !VIEWS.contains(&view) {
                self.push(
                    "schema-value",
                    format!("{path}.supporting_views[{index}]"),
                    format!("unsupported view {view:?}"),
                );
            }
            if !seen.insert(view) {
                self.push(
                    "schema-duplicate",
                    format!("{path}.supporting_views[{index}]"),
                    format!("duplicate view {view:?}"),
                );
            }
        }
    }

    fn validate_bead_refs(
        &mut self,
        value: Option<&Value>,
        path: &str,
        beads: &BTreeMap<String, BeadRecord>,
    ) {
        let Some(sequence) = value.and_then(Value::as_sequence) else {
            self.push(
                "schema-missing",
                format!("{path}.owner_decision_refs"),
                "owner_decision_refs must be an array of references, not a selected value",
            );
            return;
        };
        let mut seen = BTreeSet::new();
        for (index, item) in sequence.iter().enumerate() {
            let Some(reference) = item.as_str() else {
                self.push(
                    "schema-malformed",
                    format!("{path}.owner_decision_refs[{index}]"),
                    "owner decision reference must be a string",
                );
                continue;
            };
            if !seen.insert(reference) {
                self.push(
                    "schema-duplicate",
                    format!("{path}.owner_decision_refs[{index}]"),
                    "duplicate owner decision reference",
                );
            }
            let Some(record) = beads.get(reference) else {
                self.push(
                    "orphan-bead",
                    format!("{path}.owner_decision_refs[{index}]"),
                    format!(
                        "owner decision Bead ID {reference} is not present in the tracked export"
                    ),
                );
                continue;
            };
            let reference_path = format!("{path}.owner_decision_refs[{index}]");
            if record.status != "closed" {
                self.push(
                    "owner-decision-not-closed",
                    &reference_path,
                    format!(
                        "owner decision Bead ID {reference} has status {:?}",
                        record.status
                    ),
                );
            }
            if !record.labels.contains("decision-needed") || !record.labels.contains("human-input")
            {
                self.push(
                    "owner-decision-labels-missing",
                    &reference_path,
                    format!(
                        "owner decision Bead ID {reference} must retain decision-needed and human-input labels"
                    ),
                );
            }
            if record.issue_type != "task" && record.issue_type != "decision" {
                self.push(
                    "owner-decision-type-invalid",
                    &reference_path,
                    format!(
                        "owner decision Bead ID {reference} has unsupported issue type {:?}",
                        record.issue_type
                    ),
                );
            }
            if !record.has_close_provenance {
                self.push(
                    "owner-decision-provenance-missing",
                    &reference_path,
                    format!("owner decision Bead ID {reference} must include close provenance"),
                );
            }
        }
    }

    fn reject_policy_values(&mut self, value: &Value, path: &str) {
        match value {
            Value::String(value) if contains_policy_assignment(value) => {
                self.push(
                    "policy-value-forbidden",
                    path,
                    "test data contains a policy assignment; record a decision reference instead",
                );
            }
            Value::Sequence(sequence) => {
                for (index, item) in sequence.iter().enumerate() {
                    self.reject_policy_values(item, &format!("{path}[{index}]"));
                }
            }
            Value::Mapping(mapping) => {
                for (key, item) in mapping {
                    if let Some(key) = key.as_str() {
                        self.reject_policy_values(item, &format!("{path}.{key}"));
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_structured_locator_record(
    source: &str,
    role: &str,
    selector: &str,
    binding: LocatorBinding<'_>,
) -> Result<(), String> {
    StrictJson::new(source, MAX_NESTING_DEPTH, MAX_STRING_BYTES)
        .parse()
        .map_err(|error| format!("structured locator record is not strict JSON: {error}"))?;
    let value = yaml_serde::from_str::<Value>(source)
        .map_err(|error| format!("structured locator record is malformed: {error}"))?;
    let Some(record) = value.as_mapping() else {
        return Err("structured locator record must be an object".to_owned());
    };
    let (identity_key, expected_schema) = match role {
        "fixture" => ("fixture_id", FIXTURE_SCHEMA),
        "artifact" => ("evidence_id", EVIDENCE_SCHEMA),
        _ => return Err(format!("unsupported structured locator role {role:?}")),
    };
    let expected_keys = [
        "schema",
        "row_id",
        "case_id",
        identity_key,
        "locator_role",
        "downstream_bead",
    ];
    if record
        .keys()
        .any(|key| key.as_str().is_none_or(|key| !expected_keys.contains(&key)))
        || record.len() != expected_keys.len()
    {
        return Err("structured locator record has an unexpected schema or extra field".to_owned());
    }
    if mapping_nonempty(record, "schema") != Some(expected_schema)
        || mapping_nonempty(record, "row_id") != Some(binding.row_id)
        || mapping_nonempty(record, "case_id") != Some(binding.case_id)
        || mapping_nonempty(record, identity_key) != Some(binding.identity)
        || mapping_nonempty(record, "locator_role") != Some(role)
        || mapping_nonempty(record, "downstream_bead") != Some(binding.downstream_bead)
        || selector != binding.identity
    {
        return Err(
            "structured locator record does not exactly bind row, case, identity, role, and downstream Bead"
                .to_owned(),
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RustToken {
    Ident(String),
    Punct(char),
}

fn resolve_rust_selector(source: &str, selector: &str) -> Result<(), String> {
    let components = parse_rust_selector(selector)?;
    let tokens = lex_rust(source)?;
    let delimiters = delimiter_pairs(&tokens)?;
    if scan_rust_scope(&tokens, &delimiters, 0, tokens.len(), &[], &components)? {
        Ok(())
    } else {
        Err("no exact function or module declaration matches the selector".to_owned())
    }
}

fn scan_rust_scope(
    tokens: &[RustToken],
    delimiters: &[Option<usize>],
    start: usize,
    end: usize,
    modules: &[String],
    selector: &[String],
) -> Result<bool, String> {
    let mut index = start;
    while index < end {
        if let Some(after_attribute) = rust_attribute_end(tokens, delimiters, index)? {
            index = after_attribute;
            continue;
        }
        if let Some(after_macro) = rust_macro_end(tokens, delimiters, index)? {
            index = after_macro;
            continue;
        }

        if let Some((name, open, close)) = inline_module(tokens, delimiters, index, end)? {
            let mut declaration = modules.to_vec();
            declaration.push(name);
            if declaration == selector {
                return Ok(true);
            }
            if scan_rust_scope(tokens, delimiters, open + 1, close, &declaration, selector)? {
                return Ok(true);
            }
            index = close + 1;
            continue;
        }

        if let RustToken::Ident(keyword) = &tokens[index]
            && keyword == "fn"
            && let Some(RustToken::Ident(name)) = tokens.get(index + 1)
        {
            let mut declaration = modules.to_vec();
            declaration.push(name.clone());
            if declaration == selector {
                return Ok(true);
            }
        }

        match tokens[index] {
            RustToken::Punct('{') | RustToken::Punct('(') | RustToken::Punct('[') => {
                let close = delimiters[index].ok_or_else(|| {
                    "Rust selector source contains an unbalanced delimiter".to_owned()
                })?;
                if close >= end {
                    return Err("Rust selector scope contains an unbalanced delimiter".to_owned());
                }
                index = close + 1;
            }
            RustToken::Punct('}') | RustToken::Punct(')') | RustToken::Punct(']') => {
                return Err(
                    "Rust selector source contains an unexpected closing delimiter".to_owned(),
                );
            }
            _ => index += 1,
        }
    }
    Ok(false)
}

fn delimiter_pairs(tokens: &[RustToken]) -> Result<Vec<Option<usize>>, String> {
    let mut pairs = vec![None; tokens.len()];
    let mut stack = Vec::<(char, usize)>::new();
    for (index, token) in tokens.iter().enumerate() {
        let RustToken::Punct(punctuation) = token else {
            continue;
        };
        if is_open_delimiter(*punctuation) {
            stack.push((*punctuation, index));
        } else if let Some(expected_open) = delimiter_open_for(*punctuation) {
            let Some((open, open_index)) = stack.pop() else {
                return Err(format!(
                    "Rust selector source has an unexpected closing delimiter {punctuation:?}"
                ));
            };
            if open != expected_open {
                return Err(format!(
                    "Rust selector source has mismatched delimiters {open:?} and {punctuation:?}"
                ));
            }
            pairs[open_index] = Some(index);
            pairs[index] = Some(open_index);
        }
    }
    if let Some((open, _)) = stack.last() {
        return Err(format!(
            "Rust selector source has an unclosed delimiter {open:?}"
        ));
    }
    Ok(pairs)
}

fn rust_attribute_end(
    tokens: &[RustToken],
    delimiters: &[Option<usize>],
    index: usize,
) -> Result<Option<usize>, String> {
    if !matches!(tokens.get(index), Some(RustToken::Punct('#'))) {
        return Ok(None);
    }
    let bracket = match tokens.get(index + 1) {
        Some(RustToken::Punct('[')) => index + 1,
        Some(RustToken::Punct('!'))
            if matches!(tokens.get(index + 2), Some(RustToken::Punct('['))) =>
        {
            index + 2
        }
        _ => return Ok(None),
    };
    let close = delimiters[bracket]
        .ok_or_else(|| "Rust attribute has an unbalanced bracket delimiter".to_owned())?;
    Ok(Some(close + 1))
}

fn rust_macro_end(
    tokens: &[RustToken],
    delimiters: &[Option<usize>],
    index: usize,
) -> Result<Option<usize>, String> {
    let Some(RustToken::Ident(head)) = tokens.get(index) else {
        return Ok(None);
    };

    if head == "macro_rules" && matches!(tokens.get(index + 1), Some(RustToken::Punct('!'))) {
        let mut open = index + 2;
        if matches!(tokens.get(open), Some(RustToken::Ident(_))) {
            open += 1;
        }
        return macro_delimited_end(tokens, delimiters, open, "macro_rules definition");
    }

    let mut bang = index + 1;
    while matches!(tokens.get(bang), Some(RustToken::Punct(':')))
        && matches!(tokens.get(bang + 1), Some(RustToken::Punct(':')))
        && matches!(tokens.get(bang + 2), Some(RustToken::Ident(_)))
    {
        bang += 2;
        bang += 1;
    }
    if !matches!(tokens.get(bang), Some(RustToken::Punct('!'))) {
        return Ok(None);
    }
    if matches!(tokens.get(bang + 1), Some(RustToken::Punct('='))) {
        return Ok(None);
    }
    if is_rust_keyword(head) {
        return Ok(None);
    }
    macro_delimited_end(tokens, delimiters, bang + 1, "macro invocation")
}

fn macro_delimited_end(
    tokens: &[RustToken],
    delimiters: &[Option<usize>],
    open: usize,
    kind: &str,
) -> Result<Option<usize>, String> {
    if !matches!(
        tokens.get(open),
        Some(RustToken::Punct('{') | RustToken::Punct('(') | RustToken::Punct('['))
    ) {
        return Err(format!(
            "{kind} must use a balanced brace, parenthesis, or bracket token tree"
        ));
    }
    let close = delimiters[open].ok_or_else(|| format!("{kind} has an unbalanced token tree"))?;
    Ok(Some(close + 1))
}

fn inline_module(
    tokens: &[RustToken],
    delimiters: &[Option<usize>],
    index: usize,
    end: usize,
) -> Result<Option<(String, usize, usize)>, String> {
    if !matches!(tokens.get(index), Some(RustToken::Ident(keyword)) if keyword == "mod") {
        return Ok(None);
    }
    let Some(RustToken::Ident(name)) = tokens.get(index + 1) else {
        return Ok(None);
    };
    let Some(RustToken::Punct('{')) = tokens.get(index + 2) else {
        return Ok(None);
    };
    let open = index + 2;
    let close =
        delimiters[open].ok_or_else(|| "inline Rust module has an unbalanced body".to_owned())?;
    if close >= end {
        return Err("inline Rust module escapes its containing scope".to_owned());
    }
    Ok(Some((name.clone(), open, close)))
}

fn is_open_delimiter(punctuation: char) -> bool {
    matches!(punctuation, '{' | '(' | '[')
}

fn delimiter_open_for(punctuation: char) -> Option<char> {
    match punctuation {
        '}' => Some('{'),
        ')' => Some('('),
        ']' => Some('['),
        _ => None,
    }
}

fn is_rust_keyword(identifier: &str) -> bool {
    matches!(
        identifier,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
    )
}

fn parse_rust_selector(selector: &str) -> Result<Vec<String>, String> {
    if selector.is_empty() {
        return Err("selector is empty".to_owned());
    }
    let mut components = Vec::new();
    for component in selector.split("::") {
        if component.is_empty()
            || !component.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
            })
        {
            return Err(
                "selector must contain Rust identifier components separated by ::".to_owned(),
            );
        }
        components.push(component.to_owned());
    }
    Ok(components)
}

fn lex_rust(source: &str) -> Result<Vec<RustToken>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_rust_block_comment(bytes, index)?;
            }
            b'b' if is_raw_string_start(bytes, index, 1) => {
                index = skip_rust_raw_string(bytes, index, 1)?;
            }
            b'b' if bytes.get(index + 1) == Some(&b'"') => {
                index = skip_rust_quoted(bytes, index + 1, b'"')?;
            }
            b'r' if is_raw_string_start(bytes, index, 0) => {
                index = skip_rust_raw_string(bytes, index, 0)?;
            }
            b'"' => index = skip_rust_quoted(bytes, index, b'"')?,
            b'\'' => {
                if bytes
                    .get(index + 1)
                    .is_some_and(|byte| is_rust_ident_start(*byte))
                {
                    tokens.push(RustToken::Punct(b'\'' as char));
                    index += 1;
                } else if let Some(end) = rust_char_literal_end(bytes, index) {
                    index = end;
                } else {
                    tokens.push(RustToken::Punct(b'\'' as char));
                    index += 1;
                }
            }
            byte if is_rust_ident_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_rust_ident_continue(bytes[index]) {
                    index += 1;
                }
                tokens.push(RustToken::Ident(source[start..index].to_owned()));
            }
            byte => {
                tokens.push(RustToken::Punct(byte as char));
                index += 1;
            }
        }
    }
    Ok(tokens)
}

fn is_rust_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_rust_ident_continue(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn is_raw_string_start(bytes: &[u8], index: usize, prefix_len: usize) -> bool {
    let raw = index + prefix_len;
    if bytes.get(raw) != Some(&b'r') {
        return false;
    }
    let mut delimiter_end = raw + 1;
    while bytes.get(delimiter_end) == Some(&b'#') {
        delimiter_end += 1;
    }
    bytes.get(delimiter_end) == Some(&b'"')
}

fn skip_rust_raw_string(bytes: &[u8], index: usize, prefix_len: usize) -> Result<usize, String> {
    let raw = index + prefix_len;
    let mut delimiter_end = raw + 1;
    while bytes.get(delimiter_end) == Some(&b'#') {
        delimiter_end += 1;
    }
    if bytes.get(delimiter_end) != Some(&b'"') {
        return Err("malformed raw string prefix".to_owned());
    }
    let hashes = delimiter_end - raw - 1;
    let mut cursor = delimiter_end + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes
                .get(cursor + 1..cursor + 1 + hashes)
                .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
        {
            return Ok(cursor + 1 + hashes);
        }
        cursor += 1;
    }
    Err("unterminated raw string".to_owned())
}

fn skip_rust_quoted(bytes: &[u8], start: usize, quote: u8) -> Result<usize, String> {
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor = cursor.saturating_add(2),
            byte if byte == quote => return Ok(cursor + 1),
            b'\n' if quote == b'"' => return Err("unterminated Rust string".to_owned()),
            _ => cursor += 1,
        }
    }
    Err("unterminated Rust quoted literal".to_owned())
}

fn rust_char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    let mut escaped = false;
    while cursor < bytes.len() && bytes[cursor] != b'\n' {
        let byte = bytes[cursor];
        if byte == b'\'' && !escaped {
            return Some(cursor + 1);
        }
        escaped = byte == b'\\' && !escaped;
        cursor += 1;
    }
    None
}

fn skip_rust_block_comment(bytes: &[u8], start: usize) -> Result<usize, String> {
    let mut depth = 1usize;
    let mut cursor = start + 2;
    while cursor + 1 < bytes.len() {
        if bytes[cursor] == b'/' && bytes[cursor + 1] == b'*' {
            depth += 1;
            cursor += 2;
        } else if bytes[cursor] == b'*' && bytes[cursor + 1] == b'/' {
            depth -= 1;
            cursor += 2;
            if depth == 0 {
                return Ok(cursor);
            }
        } else {
            cursor += 1;
        }
    }
    Err("unterminated Rust block comment".to_owned())
}

fn collect_bead_records(
    source: &str,
    validator: &mut Validator<'_>,
) -> BTreeMap<String, BeadRecord> {
    let mut records = BTreeMap::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_RECORD_BYTES {
            validator.push(
                "bead-record-too-large",
                format!("beads[{index}]"),
                format!("tracked Beads record exceeds {MAX_RECORD_BYTES} bytes"),
            );
            continue;
        }
        if let Err(error) = StrictJson::new(line, MAX_NESTING_DEPTH, MAX_BEAD_STRING_BYTES).parse()
        {
            validator.push(
                "bead-export-malformed",
                format!("beads[{index}]"),
                format!("tracked Beads record is not strict JSON: {error}"),
            );
            continue;
        }
        let value = match yaml_serde::from_str::<Value>(line) {
            Ok(value) => value,
            Err(error) => {
                validator.push(
                    "bead-export-malformed",
                    format!("beads[{index}]"),
                    format!("tracked Beads record is malformed: {error}"),
                );
                continue;
            }
        };
        let Some(mapping) = value.as_mapping() else {
            validator.push(
                "bead-export-malformed",
                format!("beads[{index}]"),
                "tracked Beads record must be an object",
            );
            continue;
        };
        let Some(id) = mapping
            .get(Value::String("id".to_owned()))
            .and_then(Value::as_str)
        else {
            validator.push(
                "bead-export-malformed",
                format!("beads[{index}]"),
                "tracked Beads record must contain a string id",
            );
            continue;
        };
        if !is_stable_id(id) {
            validator.push(
                "bead-export-malformed",
                format!("beads[{index}].id"),
                format!("tracked Bead ID {id:?} is not a stable lowercase ID"),
            );
        }
        if records.contains_key(id) {
            validator.push(
                "duplicate-bead-id",
                format!("beads[{index}].id"),
                format!("duplicate tracked Bead ID {id}"),
            );
            continue;
        }
        let Some(status) = mapping_nonempty(mapping, "status") else {
            validator.push(
                "bead-export-malformed",
                format!("beads[{index}].status"),
                "tracked Beads record must contain a non-empty string status",
            );
            continue;
        };
        let Some(issue_type) = mapping_nonempty(mapping, "issue_type") else {
            validator.push(
                "bead-export-malformed",
                format!("beads[{index}].issue_type"),
                "tracked Beads record must contain a non-empty string issue_type",
            );
            continue;
        };
        let mut labels = BTreeSet::new();
        if let Some(value) = mapping.get(Value::String("labels".to_owned())) {
            let Some(items) = value.as_sequence() else {
                validator.push(
                    "bead-export-malformed",
                    format!("beads[{index}].labels"),
                    "tracked Beads labels must be an array of strings",
                );
                continue;
            };
            for (label_index, item) in items.iter().enumerate() {
                let Some(label) = item.as_str() else {
                    validator.push(
                        "bead-export-malformed",
                        format!("beads[{index}].labels[{label_index}]"),
                        "tracked Bead label must be a string",
                    );
                    continue;
                };
                if !labels.insert(label.to_owned()) {
                    validator.push(
                        "bead-export-malformed",
                        format!("beads[{index}].labels[{label_index}]"),
                        format!("duplicate tracked Bead label {label:?}"),
                    );
                }
            }
        }
        let has_close_provenance = ["created_at", "created_by", "closed_at", "close_reason"]
            .iter()
            .all(|key| mapping_nonempty(mapping, key).is_some());
        let traceability_evidence = parse_traceability_evidence(mapping, index, validator);
        records.insert(
            id.to_owned(),
            BeadRecord {
                status: status.to_owned(),
                issue_type: issue_type.to_owned(),
                labels,
                has_close_provenance,
                traceability_evidence,
            },
        );
    }
    records
}

fn parse_traceability_evidence(
    mapping: &Mapping,
    bead_index: usize,
    validator: &mut Validator<'_>,
) -> Vec<TraceabilityEvidence> {
    let Some(value) = mapping.get(Value::String("traceability_evidence".to_owned())) else {
        return Vec::new();
    };
    let Some(sequence) = value.as_sequence() else {
        validator.push(
            "bead-evidence-malformed",
            format!("beads[{bead_index}].traceability_evidence"),
            "traceability_evidence must be an array of exact structured records",
        );
        return Vec::new();
    };
    let mut records = Vec::new();
    for (record_index, item) in sequence.iter().enumerate() {
        let path = format!("beads[{bead_index}].traceability_evidence[{record_index}]");
        let Some(record) = item.as_mapping() else {
            validator.push(
                "bead-evidence-malformed",
                &path,
                "traceability evidence must be an object",
            );
            continue;
        };
        validator.reject_unknown_keys(
            record,
            &path,
            &[
                "schema",
                "row_id",
                "case_id",
                "evidence_id",
                "locator_role",
                "downstream_bead",
            ],
        );
        let get = |key: &str| mapping_nonempty(record, key).map(str::to_owned);
        let schema = get("schema");
        let row_id = get("row_id");
        let case_id = get("case_id");
        let evidence_id = get("evidence_id");
        let locator_role = get("locator_role");
        let downstream_bead = get("downstream_bead");
        if schema.as_deref() != Some(EVIDENCE_SCHEMA)
            || row_id.as_deref().is_none_or(|value| !is_stable_id(value))
            || case_id.as_deref().is_none_or(|value| !is_identity(value))
            || evidence_id
                .as_deref()
                .is_none_or(|value| !is_identity(value))
            || locator_role.as_deref() != Some("artifact")
            || downstream_bead
                .as_deref()
                .is_none_or(|value| !is_stable_id(value))
        {
            validator.push(
                "bead-evidence-malformed",
                &path,
                "traceability evidence must use the exact artifact provenance schema",
            );
            continue;
        }
        records.push(TraceabilityEvidence {
            schema: schema.expect("validated schema"),
            row_id: row_id.expect("validated row ID"),
            case_id: case_id.expect("validated case ID"),
            evidence_id: evidence_id.expect("validated evidence ID"),
            locator_role: locator_role.expect("validated locator role"),
            downstream_bead: downstream_bead.expect("validated downstream Bead"),
        });
    }
    records
}

fn mapping_nonempty<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a str> {
    mapping
        .get(Value::String(key.to_owned()))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn is_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn is_identity(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b':')
        })
}

fn is_contract_identity(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b':' | b'#')
        })
}

fn is_selector(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.' | b'/')
        })
}

fn is_policy_selecting_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "decision"
            | "decision_value"
            | "selected"
            | "selected_value"
            | "selection"
            | "selected_policy"
            | "chosen"
            | "chosen_value"
            | "choice"
            | "decision_result"
            | "policy"
            | "policy_value"
            | "effective_policy"
            | "effective_value"
            | "override"
            | "override_value"
    )
}

fn contains_policy_assignment(value: &str) -> bool {
    // Remove whitespace around assignment punctuation, then match both the
    // ordinary and selection-oriented spellings. This intentionally rejects
    // a policy assignment without interpreting the assigned value.
    let compact = value
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    [
        "decision=",
        "decision:",
        "decisionvalue=",
        "decisionvalue:",
        "selected=",
        "selected:",
        "selectedvalue=",
        "selectedvalue:",
        "selectedpolicy=",
        "selectedpolicy:",
        "chosen=",
        "chosen:",
        "chosenvalue=",
        "chosenvalue:",
        "choice=",
        "choice:",
        "ownerchoice=",
        "ownerchoice:",
        "selection=",
        "selection:",
        "policy=",
        "policy:",
        "policyvalue=",
        "policyvalue:",
        "effectivepolicy=",
        "effectivepolicy:",
        "effectivevalue=",
        "effectivevalue:",
        "override=",
        "override:",
        "overridevalue=",
        "overridevalue:",
        "selectedpolicy",
        "selectedvalue",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
}

fn bounded(value: String) -> String {
    if value.len() <= MAX_TEXT {
        return value;
    }
    const MARKER: &str = "… [truncated]";
    let mut end = MAX_TEXT.saturating_sub(MARKER.len()).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut output = value;
    output.truncate(end);
    output.push_str(MARKER);
    output
}

fn stable_digest(value: &str) -> String {
    // FNV-1a is a deterministic replay key, not a security boundary.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
