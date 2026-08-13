//! Fail-closed coverage for executable Rust lines changed from an explicit Git base.

use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const REPORT_SCHEMA: &str = "omnirepo.changed-executable-coverage.v1";
pub const FLOOR_PERCENT: u64 = 95;
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
    pub coverage_percent: u64,
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
    let rename_check = if options.head.is_some() {
        git(
            &root,
            &["diff", "--no-color", "--name-status", "-M", &base, &head, "--"],
        )?
    } else {
        git(
            &root,
            &["diff", "--no-color", "--name-status", "-M", &base, "--"],
        )?
    };
    if rename_check
        .lines()
        .any(|line| line.starts_with('R') || line.starts_with('C'))
    {
        return Err(Error::Diff("rename or copy is ambiguous".into()));
    }
    let diff = git(&root, &diff_args)?;
    if diff.len() > MAX_DIFF_BYTES {
        return Err(Error::Diff("diff exceeds size limit".into()));
    }
    let mut changed = parse_diff(&diff)?;
    let untracked = git(&root, &["ls-files", "--others", "--exclude-standard"])?;
    for raw in untracked.lines() {
        let raw = raw.trim();
        if raw.is_empty()
            || !raw.ends_with(".rs")
            || !(raw.starts_with("src/") || raw.contains("/src/"))
        {
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
            return Err(Error::Lcov(format!(
                "missing executable source record for {path}"
            )));
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
    let percent = covered
        .saturating_mul(100)
        .checked_div(total)
        .unwrap_or(100);
    let report = Report {
        schema: REPORT_SCHEMA,
        base,
        head,
        threshold_percent: FLOOR_PERCENT,
        executable_changed_lines: total,
        covered_changed_lines: covered,
        coverage_percent: percent,
        passed: total == 0 || percent >= FLOOR_PERCENT,
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

pub fn resolve_base(explicit: Option<&str>) -> Result<String, Error> {
    explicit
        .map(str::to_owned)
        .or_else(|| std::env::var("OMNIREPO_COVERAGE_BASE").ok())
        .filter(|base| !base.trim().is_empty())
        .ok_or(Error::MissingBase)
}
fn verify_revision(root: &Path, revision: &str) -> Result<String, Error> {
    if revision.is_empty() || revision.starts_with('-') {
        return Err(Error::Git("base/head is missing or unsafe".into()));
    }
    let resolved = git(
        root,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
    )?;
    let resolved = resolved.trim();
    if resolved.is_empty() || resolved.chars().any(char::is_whitespace) {
        return Err(Error::Git("revision did not resolve to one commit".into()));
    }
    Ok(resolved.to_owned())
}
fn git(root: &Path, args: &[&str]) -> Result<String, Error> {
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
    String::from_utf8(output.stdout).map_err(|e| Error::Git(e.to_string()))
}
fn safe_product_path(raw: &str) -> Result<String, Error> {
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
    if !normalized.ends_with(".rs")
        || !(normalized.starts_with("src/") || normalized.contains("/src/"))
    {
        return Err(Error::Path(format!("not product Rust source: {raw}")));
    }
    Ok(normalized)
}
fn parse_diff(diff: &str) -> Result<BTreeMap<String, BTreeSet<u64>>, Error> {
    let mut result: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    let mut path = None;
    let mut new_line = 0u64;
    let mut remaining = 0u64;
    for raw in diff.lines() {
        if raw.starts_with("diff --git ") {
            path = None;
            remaining = 0;
        } else if let Some(value) = raw.strip_prefix("+++ b/") {
            path = Some(safe_product_path(value)?);
        } else if raw.starts_with("+++ ") {
            return Err(Error::Diff("binary/deleted file encountered".into()));
        } else if raw.starts_with("@@ ") {
            if path.is_none() {
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
                    if let Some(p) = &path {
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
    use super::{FLOOR_PERCENT, REPORT_SCHEMA, Report, parse_diff};

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
    fn report_serialization_is_bounded_and_versioned() {
        let report = Report {
            schema: REPORT_SCHEMA,
            base: "base".into(),
            head: "head".into(),
            threshold_percent: FLOOR_PERCENT,
            executable_changed_lines: 0,
            covered_changed_lines: 0,
            coverage_percent: 100,
            passed: true,
            lines: Vec::new(),
        };
        let json = report.json().expect("small report");
        assert!(json.contains(REPORT_SCHEMA));
        assert!(json.contains("\"base\":\"base\""));
    }
}
