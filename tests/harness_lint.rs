use std::{fs, io, path::Path};

use tempfile::tempdir;

#[derive(Debug, Eq, PartialEq)]
struct SleepFinding {
    line: usize,
}

const REQUIRED_FIELDS: &[&str] = &[
    "Fixture owner:",
    "Fault point:",
    "Barrier:",
    "Seed/replay ID:",
    "Capability check:",
    "Evidence bundle path:",
];

fn wall_clock_sleep_findings(path: &Path) -> io::Result<Vec<SleepFinding>> {
    let source = fs::read_to_string(path)?;
    let imported_sleep = source.lines().any(|line| {
        let compact: String = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        compact.starts_with("usestd::thread::sleep")
            || compact.starts_with("use std::thread::sleep")
    });

    Ok(source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let code = line.split_once("//").map_or(line, |(code, _)| code);
            let compact: String = code
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            let is_wall_clock_sleep = compact.contains("std::thread::sleep(")
                || compact.contains("thread::sleep(")
                || compact.contains("tokio::time::sleep(")
                || (imported_sleep && compact.contains("sleep("));
            is_wall_clock_sleep.then_some(SleepFinding { line: index + 1 })
        })
        .collect())
}

fn rust_code_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = None;
    for line in markdown.lines() {
        if line.trim() == "```rust" {
            current = Some(String::new());
        } else if line.trim() == "```" {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
        } else if let Some(block) = current.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    blocks
}

fn section<'a>(markdown: &'a str, title: &str) -> &'a str {
    let heading = format!("## {title}");
    let start = markdown
        .find(&heading)
        .unwrap_or_else(|| panic!("missing documentation section {title:?}"));
    let body = &markdown[start + heading.len()..];
    body.find("\n## ").map_or(body, |end| &body[..end])
}

#[test]
fn wall_clock_sleep_in_temporary_lifecycle_fixture_is_rejected() {
    let temporary = tempdir().expect("temporary fixture should be created");
    let source = temporary.path().join("intentional_sleep.rs");
    fs::write(
        &source,
        "fn lifecycle_fixture() { std::thread::sleep(std::time::Duration::from_millis(1)); }\n",
    )
    .expect("temporary fixture should be written");

    let findings = wall_clock_sleep_findings(&source).expect("fixture should be linted");

    assert_eq!(findings, vec![SleepFinding { line: 1 }]);
}

#[test]
fn canonical_examples_have_no_wall_clock_sleep_or_retry() {
    let documentation =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/harness-patterns.md"))
            .expect("canonical harness documentation should exist");
    let blocks = rust_code_blocks(&documentation);
    assert_eq!(
        blocks.len(),
        4,
        "one executable example is required per pattern"
    );

    for (index, block) in blocks.iter().enumerate() {
        let temporary = tempdir().expect("temporary lint directory should be created");
        let source = temporary.path().join(format!("pattern-{index}.rs"));
        fs::write(&source, block).expect("example source should be written");
        let findings = wall_clock_sleep_findings(&source).expect("example should be linted");
        assert!(
            findings.is_empty(),
            "pattern {index} contains wall-clock sleep: {findings:?}"
        );
        assert!(
            !block.contains("retry"),
            "pattern {index} contains a retry loop"
        );
    }
}

#[test]
fn canonical_examples_declare_the_primitive_contract() {
    let documentation =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/harness-patterns.md"))
            .expect("canonical harness documentation should exist");

    for title in [
        "Component",
        "Process tree",
        "Crash/restart",
        "Concurrent fleet",
    ] {
        let body = section(&documentation, title);
        for field in REQUIRED_FIELDS {
            assert!(body.contains(field), "{title} is missing {field}");
        }
    }
    assert!(section(&documentation, "Component").contains("LifecycleFixture::create"));
    assert!(section(&documentation, "Process tree").contains("FakeExecutable::spawn"));
    assert!(section(&documentation, "Crash/restart").contains("CrashableParent::spawn"));
    assert!(section(&documentation, "Concurrent fleet").contains("ConcurrentRunControl::launch"));
    assert!(!documentation.contains("runner"));
    assert!(!documentation.contains("journey"));
}
