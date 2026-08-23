//! Fail-closed coverage for executable Rust lines changed from an explicit Git base.

use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const REPORT_SCHEMA: &str = "omnirepo.changed-executable-coverage.v1";
pub const FLOOR_PERCENT: u64 = 80;
const MAX_DIFF_BYTES: usize = 16 * 1024 * 1024;
const MAX_LCOV_BYTES: usize = 64 * 1024 * 1024;
const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Git(String),
    MissingBase,
    Diff(String),
    Lcov(String),
    Path(String),
    Io { path: PathBuf, message: String },
    ReportTooLarge,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(s) => write!(f, "git comparison failed: {s}"),
            Self::MissingBase => {
                f.write_str("changed coverage requires --base or OMNIREPO_COVERAGE_BASE")
            }
            Self::Diff(s) => write!(f, "changed-line diff is invalid: {s}"),
            Self::Lcov(s) => write!(f, "LCOV is invalid: {s}"),
            Self::Path(s) => write!(f, "unsafe coverage path: {s}"),
            Self::Io { path, message } => write!(f, "cannot write {}: {message}", path.display()),
            Self::ReportTooLarge => f.write_str("changed coverage report exceeds its size limit"),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Options {
    pub repository_root: PathBuf,
    pub base: String,
    pub head: Option<String>,
    pub lcov_path: PathBuf,
    pub report_path: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Line {
    pub path: String,
    pub line: u64,
    pub status: &'static str,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Report {
    pub schema: &'static str,
    pub base: String,
    pub head: String,
    pub threshold_percent: u64,
    pub executable_changed_lines: u64,
    pub covered_changed_lines: u64,
    /// Informational floor percentage; `None` for an explicit zero sample.
    pub coverage_percent: Option<u64>,
    /// Non-lossy ratio `covered/total`; the gate result derives from the exact
    /// comparison, never from the rounded percentage.
    pub coverage_ratio: String,
    pub passed: bool,
    pub lines: Vec<Line>,
}
impl Report {
    pub fn json(&self) -> Result<String, Error> {
        let json = serde_json::to_string(self).map_err(|e| Error::Diff(e.to_string()))?;
        if json.len() > MAX_REPORT_BYTES {
            return Err(Error::ReportTooLarge);
        }
        Ok(json)
    }
}

/// Exact 80% threshold: `covered * 100 >= total * 80`. Overflow fails closed.
/// A zero sample (no executable changed lines) passes explicitly.
pub fn passes_threshold(covered: u64, total: u64) -> bool {
    if total == 0 {
        return true;
    }
    match (covered.checked_mul(100), total.checked_mul(FLOOR_PERCENT)) {
        (Some(numerator), Some(denominator)) => numerator >= denominator,
        _ => false,
    }
}

/// Informational floor percentage; `None` for a zero sample.
pub fn coverage_percent(covered: u64, total: u64) -> Option<u64> {
    if total == 0 {
        return None;
    }
    covered
        .checked_mul(100)
        .and_then(|value| value.checked_div(total))
}

pub fn evaluate(options: &Options) -> Result<Report, Error> {
    let root = options
        .repository_root
        .canonicalize()
        .map_err(|e| Error::Git(e.to_string()))?;
    let base = verify_revision(&root, &options.base)?;
    let head = verify_revision(&root, options.head.as_deref().unwrap_or("HEAD"))?;
    let diff_args = if options.head.is_some() {
        vec![
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            &base,
            &head,
            "--",
        ]
    } else {
        vec![
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            &base,
            "--",
        ]
    };
    // NUL-safe rename/copy inventory: maps old product path -> new path and
    // fails closed when a target has more than one source. The changed-line
    // diff itself runs without rename detection, so a rename is a delete under
    // the old path plus an add under the new path; only the new-side added
    // lines count, exactly once.
    let name_status_args = if options.head.is_some() {
        vec![
            "diff",
            "--no-color",
            "--name-status",
            "-M",
            "-z",
            &base,
            &head,
            "--",
        ]
    } else {
        vec![
            "diff",
            "--no-color",
            "--name-status",
            "-M",
            "-z",
            &base,
            "--",
        ]
    };
    let renames = parse_name_status(&git_bytes(&root, &name_status_args)?)?;
    let diff = git_text(&root, &diff_args)?;
    if diff.len() > MAX_DIFF_BYTES {
        return Err(Error::Diff("diff exceeds size limit".into()));
    }
    let mut changed = parse_diff(&diff)?;
    // Defense: the new side of a rename must never still carry the old path.
    if let Some(old_path) = changed
        .keys()
        .find(|path| renames.contains_key(path.as_str()))
    {
        return Err(Error::Diff(format!(
            "renamed source path still received added lines: {old_path}"
        )));
    }
    // NUL-safe untracked product files count as changed lines.
    let untracked = git_bytes(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for raw in untracked.split(|byte| *byte == 0) {
        let raw = String::from_utf8_lossy(raw);
        let raw = raw.trim();
        if raw.is_empty() || !raw.ends_with(".rs") || !raw.starts_with("src/") {
            continue;
        }
        let path = safe_product_path(raw)?;
        let text = fs::read_to_string(root.join(&path))
            .map_err(|e| Error::Diff(format!("untracked {raw}: {e}")))?;
        changed
            .entry(path)
            .or_default()
            .extend(1..=text.lines().count() as u64);
    }
    let lcov = fs::read_to_string(&options.lcov_path).map_err(|e| Error::Lcov(e.to_string()))?;
    if lcov.len() > MAX_LCOV_BYTES {
        return Err(Error::Lcov("input exceeds size limit".into()));
    }
    let records = parse_lcov(&root, &lcov)?;
    let mut lines = Vec::new();
    for (path, numbers) in changed {
        let Some(record) = records.get(&path) else {
            // The official LCOV only emits records for files with at least one
            // executable line. A changed file made only of declarations or
            // comments contributes no executable changed lines and is skipped;
            // a changed file with real code but no record fails closed.
            let text = fs::read_to_string(root.join(&path))
                .map_err(|e| Error::Lcov(format!("cannot read {path}: {e}")))?;
            if file_has_executable_code(&text) {
                return Err(Error::Lcov(format!(
                    "missing executable source record for {path}"
                )));
            }
            continue;
        };
        for number in numbers {
            if let Some(hits) = record.get(&number) {
                lines.push(Line {
                    path: path.clone(),
                    line: number,
                    status: if *hits > 0 { "covered" } else { "uncovered" },
                });
            }
        }
    }
    lines.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    let total = lines.len() as u64;
    let covered = lines.iter().filter(|line| line.status == "covered").count() as u64;
    let report = Report {
        schema: REPORT_SCHEMA,
        base,
        head,
        threshold_percent: FLOOR_PERCENT,
        executable_changed_lines: total,
        covered_changed_lines: covered,
        coverage_percent: coverage_percent(covered, total),
        coverage_ratio: format!("{covered}/{total}"),
        passed: passes_threshold(covered, total),
        lines,
    };
    if let Some(path) = &options.report_path {
        fs::write(path, report.json()?).map_err(|e| Error::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
    }
    Ok(report)
}

/// Git's all-zero object id: the sentinel a push event carries in place of
/// a previous commit when it creates a ref.  It names no commit, so it is
/// an absent base rather than a revision to resolve; without this it
/// reaches `git rev-parse` and fails closed with an opaque object-name
/// error instead of naming the missing base.
fn is_null_object_id(value: &str) -> bool {
    let value = value.trim();
    value.len() >= 40 && value.chars().all(|character| character == '0')
}

pub fn resolve_base(explicit: Option<&str>) -> Result<String, Error> {
    explicit
        .map(str::to_owned)
        .or_else(|| std::env::var("OMNIREPO_COVERAGE_BASE").ok())
        .filter(|base| !base.trim().is_empty())
        .filter(|base| !is_null_object_id(base))
        .ok_or(Error::MissingBase)
}
fn verify_revision(root: &Path, revision: &str) -> Result<String, Error> {
    if revision.is_empty() || revision.starts_with('-') {
        return Err(Error::Git("base/head is missing or unsafe".into()));
    }
    let resolved = git_text(
        root,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
    )?;
    let resolved = resolved.trim();
    if resolved.is_empty() || resolved.chars().any(char::is_whitespace) {
        return Err(Error::Git("revision did not resolve to one commit".into()));
    }
    Ok(resolved.to_owned())
}
fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| Error::Git(e.to_string()))?;
    if !output.status.success() {
        return Err(Error::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(output.stdout)
}
fn git_text(root: &Path, args: &[&str]) -> Result<String, Error> {
    let bytes = git_bytes(root, args)?;
    String::from_utf8(bytes).map_err(|e| Error::Git(e.to_string()))
}
/// Conservative probe: does the source text contain a line that is not a
/// comment, attribute, blank, brace, module/use declaration, or item header?
/// The official LCOV omits files with no executable lines; such files must
/// not trigger the fail-closed missing-record rule, while real code must.
fn file_has_executable_code(text: &str) -> bool {
    let mut in_block_comment = false;
    // A multi-line `use` declaration (re-exports with braces) declares without
    // executing; consume it until its terminating semicolon.
    let mut in_use_span = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if in_use_span {
            if line.ends_with(';') {
                in_use_span = false;
            }
            continue;
        }
        if in_block_comment {
            if let Some(end) = line.find("*/") {
                in_block_comment = false;
                let rest = line[end + 2..].trim();
                if rest.is_empty() || rest.starts_with("//") {
                    continue;
                }
                return true;
            }
            continue;
        }
        if line.starts_with("/*") {
            if !line.contains("*/") {
                in_block_comment = true;
            }
            continue;
        }
        if line.starts_with("//")
            || line.starts_with("#!")
            || line.starts_with("#[")
            || line == "{"
            || line == "}"
            || line.ends_with(';') && is_declaration_line(line)
            || is_empty_item_line(line)
        {
            continue;
        }
        if first_keyword(line) == Some("use") {
            if !line.ends_with(';') {
                in_use_span = true;
            }
            continue;
        }
        return true;
    }
    false
}

/// The first significant keyword of a line after visibility/qualifier tokens.
fn first_keyword(line: &str) -> Option<&str> {
    let mut words = line.split_whitespace();
    let mut first = words.next().unwrap_or("");
    while matches!(
        first,
        "pub" | "pub(crate)" | "pub(super)" | "pub(in" | "async" | "unsafe" | "default" | "extern"
    ) {
        first = words.next().unwrap_or("");
    }
    if first.is_empty() { None } else { Some(first) }
}

/// True for a single-line item with an empty body such as `pub enum Y {}`,
/// `struct X {}`, or `fn proto() {}`; these declare without executing.
fn is_empty_item_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !(trimmed.ends_with("{}") || trimmed.ends_with("{ }")) {
        return false;
    }
    let head = trimmed[..trimmed.len() - 1].trim_end();
    let mut words = head.split_whitespace();
    let mut first = words.next().unwrap_or("");
    while matches!(
        first,
        "pub" | "pub(crate)" | "pub(super)" | "pub(in" | "async" | "unsafe" | "default" | "extern"
    ) {
        first = words.next().unwrap_or("");
    }
    matches!(
        first,
        "struct" | "enum" | "trait" | "type" | "impl" | "fn" | "mod"
    )
}

/// True when a `;`-terminated line is a pure item declaration rather than a
/// statement: `use`, `mod`, item types, `fn` prototypes, constants, and
/// `macro_rules!` all declare without executing.
fn is_declaration_line(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let mut first = words.next().unwrap_or("");
    while matches!(
        first,
        "pub" | "pub(crate)" | "pub(super)" | "pub(in" | "async" | "unsafe" | "default" | "extern"
    ) {
        first = words.next().unwrap_or("");
    }
    matches!(
        first,
        "use"
            | "mod"
            | "struct"
            | "enum"
            | "type"
            | "trait"
            | "static"
            | "const"
            | "fn"
            | "macro_rules"
            | "impl"
    )
}

/// Parse a NUL-delimited `git diff --name-status -M -z` stream into a rename
/// map (old product path -> new product path). Records are split on NUL bytes
/// so any valid Git path (including newlines and tabs) stays one record.
/// A target reached from more than one source is ambiguous and fails closed.
fn parse_name_status(bytes: &[u8]) -> Result<BTreeMap<String, String>, Error> {
    let mut map = BTreeMap::new();
    let mut seen_targets = BTreeSet::new();
    let mut records = bytes.split(|byte| *byte == 0);
    while let Some(status) = records.next() {
        if status.is_empty() {
            continue;
        }
        let status = std::str::from_utf8(status)
            .map_err(|_| Error::Diff("non-UTF-8 name-status record".into()))?;
        let kind = status.as_bytes().first().copied().unwrap_or(0);
        let Some(old_raw) = records.next() else {
            return Err(Error::Diff("name-status record lacks a path".into()));
        };
        if kind != b'R' && kind != b'C' {
            continue;
        }
        let Some(new_raw) = records.next() else {
            return Err(Error::Diff("rename record lacks a target path".into()));
        };
        let old = String::from_utf8_lossy(old_raw).trim().to_owned();
        let new = String::from_utf8_lossy(new_raw).trim().to_owned();
        if old.is_empty() || new.is_empty() || old == new {
            return Err(Error::Diff("malformed rename record".into()));
        }
        // Only product Rust sources participate in the changed-line mapping;
        // non-product renames (docs, configs) are irrelevant to LCOV lookup.
        if safe_product_path(&old).is_err() || safe_product_path(&new).is_err() {
            continue;
        }
        if !seen_targets.insert(new.clone()) {
            return Err(Error::Diff(format!(
                "ambiguous rename: multiple sources resolve to {new}"
            )));
        }
        if map.insert(old.clone(), new).is_some() {
            return Err(Error::Diff(format!("duplicate rename source: {old}")));
        }
    }
    Ok(map)
}
/// Classify a repo-relative path: `Some` for product Rust sources, `None`
/// for non-product files (skipped, never counted), and fail closed for
/// unsafe paths such as absolute or parent-traversing components.
/// Product scope matches the official coverage gate: the root package's
/// `src/` tree only, excluding test modules (`unit_tests.rs`, `*_tests.rs`,
/// `tests.rs`) and tool crates, which the official LCOV never measures.
fn classify_product_path(raw: &str) -> Result<Option<String>, Error> {
    let path = Path::new(raw);
    if path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::Path(raw.into()));
    }
    let normalized = path.to_string_lossy().replace('\\', "/");
    if !normalized.ends_with(".rs") || !normalized.starts_with("src/") {
        return Ok(None);
    }
    // Test modules never appear in the official LCOV: colocated test files
    // (unit_tests.rs, *_tests.rs, tests.rs) and test-directory subtrees
    // (a `tests/` component under src/).
    let file_name = normalized.rsplit('/').next().unwrap_or("");
    if file_name == "unit_tests.rs" || file_name == "tests.rs" || file_name.ends_with("_tests.rs") {
        return Ok(None);
    }
    if normalized
        .strip_prefix("src/")
        .unwrap_or("")
        .split('/')
        .any(|component| component == "tests")
    {
        return Ok(None);
    }
    Ok(Some(normalized))
}
fn safe_product_path(raw: &str) -> Result<String, Error> {
    match classify_product_path(raw)? {
        Some(path) => Ok(path),
        None => Err(Error::Path(format!("not product Rust source: {raw}"))),
    }
}
enum NewSide {
    /// No new-side path has been seen for the current `diff --git` record.
    Absent,
    /// A product Rust source; added lines are counted for it.
    Product(String),
    /// Deleted or non-product file; hunks are consumed but never counted.
    Skip,
}
fn parse_diff(diff: &str) -> Result<BTreeMap<String, BTreeSet<u64>>, Error> {
    let mut result: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    let mut path = NewSide::Absent;
    let mut new_line = 0u64;
    let mut remaining = 0u64;
    for raw in diff.lines() {
        if raw.starts_with("diff --git ") {
            path = NewSide::Absent;
            remaining = 0;
        } else if let Some(value) = raw.strip_prefix("+++ b/") {
            path = match classify_product_path(value)? {
                Some(product) => NewSide::Product(product),
                None => NewSide::Skip, // non-product file: hunks never count
            };
        } else if raw.starts_with("+++ ") {
            // The new side is /dev/null: a pure deletion or the deleted half
            // of a rename. Deletions have no new-side executable lines and
            // must not inflate the sample.
            path = NewSide::Skip;
        } else if raw.starts_with("@@ ") {
            if matches!(path, NewSide::Absent) {
                return Err(Error::Diff("hunk has no new-side path".into()));
            }
            let plus = raw
                .split_whitespace()
                .find(|v| v.starts_with('+'))
                .ok_or_else(|| Error::Diff("malformed hunk".into()))?;
            let range = plus[1..].split(',').collect::<Vec<_>>();
            new_line = range[0]
                .parse()
                .map_err(|_| Error::Diff("malformed hunk line".into()))?;
            remaining = range
                .get(1)
                .unwrap_or(&"1")
                .parse()
                .map_err(|_| Error::Diff("malformed hunk count".into()))?;
        } else if remaining > 0 {
            let marker = raw
                .as_bytes()
                .first()
                .copied()
                .ok_or_else(|| Error::Diff("empty hunk line".into()))?;
            match marker {
                b'+' => {
                    if let NewSide::Product(p) = &path {
                        result.entry(p.clone()).or_default().insert(new_line);
                    }
                    new_line += 1;
                    remaining -= 1;
                }
                b' ' => {
                    new_line += 1;
                    remaining -= 1;
                }
                b'-' => {}
                _ => return Err(Error::Diff("malformed hunk body".into())),
            }
        }
    }
    if remaining != 0 {
        return Err(Error::Diff("truncated hunk".into()));
    }
    Ok(result)
}
fn parse_lcov(root: &Path, text: &str) -> Result<BTreeMap<String, BTreeMap<u64, u64>>, Error> {
    let mut records = BTreeMap::new();
    let mut current = None;
    for (index, raw) in text.lines().enumerate() {
        if let Some(value) = raw.strip_prefix("SF:") {
            if current.is_some() {
                return Err(Error::Lcov(format!("nested SF at line {}", index + 1)));
            }
            let path = Path::new(value.trim());
            let relative = if path.is_absolute() {
                path.strip_prefix(root)
                    .map_err(|_| Error::Path(value.into()))?
            } else {
                path
            };
            current = Some((
                safe_product_path(&relative.to_string_lossy())?,
                BTreeMap::new(),
            ));
        } else if let Some(value) = raw.strip_prefix("DA:") {
            let (line, hits) = value
                .split_once(',')
                .ok_or_else(|| Error::Lcov(format!("malformed DA at line {}", index + 1)))?;
            let line = line
                .parse::<u64>()
                .map_err(|_| Error::Lcov(format!("malformed DA line {}", index + 1)))?;
            let hits = hits
                .split(',')
                .next()
                .unwrap_or("")
                .parse::<u64>()
                .map_err(|_| Error::Lcov(format!("malformed DA hits at line {}", index + 1)))?;
            let (_, map) = current
                .as_mut()
                .ok_or_else(|| Error::Lcov(format!("DA outside SF at line {}", index + 1)))?;
            if map.insert(line, hits).is_some() {
                return Err(Error::Lcov(format!("duplicate DA at line {}", index + 1)));
            }
        } else if raw == "end_of_record" {
            let (path, map) = current
                .take()
                .ok_or_else(|| Error::Lcov(format!("end without SF at line {}", index + 1)))?;
            if records.insert(path.clone(), map).is_some() {
                return Err(Error::Lcov(format!("duplicate SF for {path}")));
            }
        } else if raw.is_empty()
            || raw.starts_with("TN:")
            || raw.starts_with("FN:")
            || raw.starts_with("FNDA:")
            || raw.starts_with("FNF:")
            || raw.starts_with("FNH:")
            || raw.starts_with("BRDA:")
            || raw.starts_with("BRF:")
            || raw.starts_with("BRH:")
            || raw.starts_with("LH:")
            || raw.starts_with("LF:")
        {
            continue;
        } else {
            return Err(Error::Lcov(format!("unknown record at line {}", index + 1)));
        }
    }
    if current.is_some() {
        return Err(Error::Lcov("unterminated SF record".into()));
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::{
        Error, FLOOR_PERCENT, REPORT_SCHEMA, Report, parse_diff, parse_name_status, resolve_base,
    };

    #[test]
    fn diff_parser_keeps_only_added_new_side_lines() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,3 @@\n old\n-old\n+new\n+another\n";
        let parsed = parse_diff(diff).expect("valid diff");
        assert_eq!(
            parsed["src/a.rs"].iter().copied().collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn diff_parser_keeps_lines_from_a_new_file() {
        let diff = "diff --git a/src/main.rs b/src/main.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/main.rs\n@@ -0,0 +1,3 @@\n+fn main() {\n+    println!(\"changed\");\n+}\n";
        let parsed = parse_diff(diff).expect("valid new-file diff");
        assert_eq!(
            parsed["src/main.rs"].iter().copied().collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn diff_parser_skips_deleted_files_without_inflating() {
        // A pure deletion has only a /dev/null new side and minus lines.
        let diff = "diff --git a/src/gone.rs b/src/gone.rs\ndeleted file mode 100644\n--- a/src/gone.rs\n+++ /dev/null\n@@ -1,3 +0,0 @@\n-fn gone() {}\n-    println!(\"bye\");\n-}\n";
        let parsed = parse_diff(diff).expect("valid deletion diff");
        assert!(parsed.is_empty(), "deletions must not inflate: {parsed:?}");
    }

    #[test]
    fn diff_parser_handles_a_rename_without_detection_as_added_new_side() {
        // Without -M, a rename is a delete under the old path plus an add under
        // the new path; only the new-side added lines may count.
        let diff = "diff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n--- a/src/old.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-fn old() {}\n-}\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,3 @@\n+fn new() {\n+    println!(\"renamed\");\n+}\n";
        let parsed = parse_diff(diff).expect("valid rename diff");
        let new_lines = parsed.get("src/new.rs").expect("new path present");
        assert_eq!(new_lines.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(!parsed.contains_key("src/old.rs"));
    }

    #[test]
    fn name_status_parses_renames_nul_safely() {
        let records = b"R100\0src/old.rs\0src/new.rs\0";
        let map = parse_name_status(records).expect("valid rename record");
        assert_eq!(
            map.get("src/old.rs").map(String::as_str),
            Some("src/new.rs")
        );
    }

    #[test]
    fn name_status_parses_paths_containing_newlines() {
        // NUL-safety: a path with an embedded newline must stay one record.
        let records = b"R100\0src/old\nline.rs\0src/new\nline.rs\0";
        let map = parse_name_status(records).expect("NUL-safe rename record");
        assert_eq!(
            map.get("src/old\nline.rs").map(String::as_str),
            Some("src/new\nline.rs")
        );
    }

    #[test]
    fn name_status_rejects_ambiguous_rename_targets() {
        // Two different sources resolving to the same target cannot be mapped.
        let records = b"R100\0src/a.rs\0src/b.rs\0R100\0src/c.rs\0src/b.rs\0";
        let error = parse_name_status(records).expect_err("ambiguous rename must fail");
        assert!(error.to_string().contains("ambiguous"));
    }

    #[test]
    fn name_status_ignores_non_product_renames() {
        let records =
            b"R100\0README.md\0docs/README.md\0R100\0src/a.rs\0src/b.rs\0R100\0tools/omnirepo-dev/src/x.rs\0tools/omnirepo-dev/src/y.rs\0";
        let map = parse_name_status(records).expect("non-product renames are ignored");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("src/a.rs").map(String::as_str), Some("src/b.rs"));
    }

    #[test]
    fn diff_parser_skips_tool_crate_and_test_files() {
        // Tool crates and test files are outside the official coverage scope;
        // their hunks are consumed but never counted.
        let diff = "diff --git a/tools/omnirepo-dev/src/tool.rs b/tools/omnirepo-dev/src/tool.rs\n--- a/tools/omnirepo-dev/src/tool.rs\n+++ b/tools/omnirepo-dev/src/tool.rs\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\ndiff --git a/tests/some_test.rs b/tests/some_test.rs\n--- a/tests/some_test.rs\n+++ b/tests/some_test.rs\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\n";
        let parsed = parse_diff(diff).expect("valid out-of-scope diff");
        assert!(
            parsed.is_empty(),
            "out-of-scope files must not count: {parsed:?}"
        );
    }

    #[test]
    fn diff_parser_skips_colocated_test_modules() {
        // Colocated test modules (unit_tests.rs, *_tests.rs, tests.rs) are
        // never measured by the official LCOV and must not enter the sample.
        let diff = "diff --git a/src/configuration/unit_tests.rs b/src/configuration/unit_tests.rs\n--- a/src/configuration/unit_tests.rs\n+++ b/src/configuration/unit_tests.rs\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\ndiff --git a/src/platform/authority/tests.rs b/src/platform/authority/tests.rs\n--- a/src/platform/authority/tests.rs\n+++ b/src/platform/authority/tests.rs\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\ndiff --git a/src/repository/state_tests.rs b/src/repository/state_tests.rs\n--- a/src/repository/state_tests.rs\n+++ b/src/repository/state_tests.rs\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\n";
        let parsed = parse_diff(diff).expect("valid test-module diff");
        assert!(parsed.is_empty(), "test modules must not count: {parsed:?}");
    }

    #[test]
    fn executable_code_probe_distinguishes_declarations_from_code() {
        use super::file_has_executable_code;
        // Pure declaration/comment files are omitted from the official LCOV
        // and must not demand a record.
        assert!(!file_has_executable_code(
            "//! docs\n#![allow(dead_code)]\nmod run_record;\n"
        ));
        assert!(!file_has_executable_code(
            "pub mod snapshot;\n#[cfg(test)]\nmod snapshot_tests;\n"
        ));
        assert!(!file_has_executable_code(
            "/* block\n * comment\n */\npub struct X;\npub enum Y {}\npub type Z = u8;\nuse std::path::Path;\n"
        ));
        assert!(!file_has_executable_code(
            "pub fn proto();\nconst N: u64 = 1;\n"
        ));
        // Real code demands a record.
        assert!(file_has_executable_code(
            "fn main() {\n    println!(\"x\");\n}\n"
        ));
        assert!(file_has_executable_code("let x = 1;\n"));
        assert!(file_has_executable_code("pub fn foo() { let _ = 1; }\n"));
        assert!(!file_has_executable_code("pub fn empty() {}\n"));
        assert!(file_has_executable_code(
            "impl Foo {\n    fn bar(&self) {}\n}\n"
        ));
        // Multi-line re-export spans declare without executing.
        assert!(!file_has_executable_code(
            "pub(crate) use authority::{\n    MutationIntent, PathError, sync_file,\n};\n"
        ));
    }

    #[test]
    fn classify_keeps_only_product_source_files() {
        use super::classify_product_path;
        assert!(
            classify_product_path("src/configuration/mod.rs")
                .unwrap()
                .is_some()
        );
        assert!(classify_product_path("src/main.rs").unwrap().is_some());
        assert!(
            classify_product_path("src/configuration/unit_tests.rs")
                .unwrap()
                .is_none()
        );
        assert!(
            classify_product_path("src/repository/state_tests.rs")
                .unwrap()
                .is_none()
        );
        assert!(
            classify_product_path("src/platform/authority/tests.rs")
                .unwrap()
                .is_none()
        );
        assert!(
            classify_product_path("src/platform/authority/tests/coverage_tests/adapters.rs")
                .unwrap()
                .is_none()
        );
        assert!(
            classify_product_path("tools/omnirepo-dev/src/tool.rs")
                .unwrap()
                .is_none()
        );
        assert!(classify_product_path("src/mod.rs").unwrap().is_some());
    }

    #[test]
    fn zero_changed_lines_are_reported_explicitly_and_pass() {
        assert!(super::passes_threshold(0, 0));
        assert_eq!(super::coverage_percent(0, 0), None);
    }

    #[test]
    fn exact_threshold_arithmetic_never_truncates() {
        // 79/99 is 79.79%: truncating would say 79 and still fail, but the
        // exact comparison must be the authority in both directions.
        assert!(!super::passes_threshold(79, 99));
        assert!(super::passes_threshold(80, 100));
        // 880/1100 is exactly 80%; 879/1100 is 79.90%.
        assert!(!super::passes_threshold(879, 1100));
        assert!(super::passes_threshold(880, 1100));
        assert_eq!(super::coverage_percent(880, 1100), Some(80));
    }

    #[test]
    fn exact_threshold_arithmetic_fails_closed_on_overflow() {
        assert!(!super::passes_threshold(u64::MAX, u64::MAX));
        assert!(!super::passes_threshold(u64::MAX, 1));
        assert_eq!(super::coverage_percent(u64::MAX, u64::MAX), None);
    }

    #[test]
    fn report_serialization_is_bounded_and_versioned() {
        let report = Report {
            schema: REPORT_SCHEMA,
            base: "base".into(),
            head: "head".into(),
            threshold_percent: FLOOR_PERCENT,
            executable_changed_lines: 0,
            covered_changed_lines: 0,
            coverage_percent: None,
            coverage_ratio: "0/0".into(),
            passed: true,
            lines: Vec::new(),
        };
        let json = report.json().expect("small report");
        assert!(json.contains(REPORT_SCHEMA));
        assert!(json.contains("\"base\":\"base\""));
        assert!(json.contains("\"coverage_percent\":null"));
        assert!(json.contains("\"coverage_ratio\":\"0/0\""));
    }

    #[test]
    fn a_created_ref_reports_a_missing_base_not_an_object_name_failure() {
        // A push that creates a ref carries git's all-zero object id in
        // place of a previous commit.  It names no commit, so the gate
        // must fail closed naming the missing base rather than handing
        // the sentinel to `git rev-parse`.
        let null = "0".repeat(40);
        assert!(matches!(resolve_base(Some(&null)), Err(Error::MissingBase)));
        assert!(matches!(resolve_base(Some("")), Err(Error::MissingBase)));
        assert!(matches!(resolve_base(None), Err(Error::MissingBase)));
        // A real revision is still returned verbatim.
        assert_eq!(
            resolve_base(Some("a28690bb40389db2fdeaca07238be7d2c525a83b")).expect("base"),
            "a28690bb40389db2fdeaca07238be7d2c525a83b"
        );
    }
}
