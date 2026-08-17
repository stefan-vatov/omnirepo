use omnirepo_dev::changed_coverage::{FLOOR_PERCENT, REPORT_SCHEMA, Report};
use omnirepo_test_support::lifecycle_fixture::{FixtureSpec, LifecycleFixture};
use serde::Deserialize;
use std::path::PathBuf;

#[test]
fn report_has_explicit_comparison_identity_and_floor() {
    let report = Report {
        schema: REPORT_SCHEMA,
        base: "base-sha".into(),
        head: "head-sha".into(),
        threshold_percent: FLOOR_PERCENT,
        executable_changed_lines: 0,
        covered_changed_lines: 0,
        coverage_percent: None,
        coverage_ratio: "0/0".into(),
        passed: true,
        lines: Vec::new(),
    };
    let json = report.json().expect("bounded report");
    assert!(json.contains("\"base\":\"base-sha\""));
    assert!(json.contains("\"head\":\"head-sha\""));
    assert!(json.contains("\"threshold_percent\":80"));
    assert!(json.contains("\"coverage_percent\":null"));
    assert!(json.contains("\"coverage_ratio\":\"0/0\""));
}

#[derive(Debug, Deserialize)]
struct ChangedReport {
    schema: String,
    base: String,
    head: String,
    executable_changed_lines: u64,
    covered_changed_lines: u64,
    coverage_percent: Option<u64>,
    coverage_ratio: String,
    passed: bool,
}

fn git(_fixture: &LifecycleFixture, root: &std::path::Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("git {arguments:?} should start: {error}"));
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_evaluator(
    root: &std::path::Path,
    lcov: &std::path::Path,
    extra: &[&str],
) -> omnirepo_dev::CommandOutput {
    let mut arguments = vec![
        "changed-coverage".to_owned(),
        "--repo-root".to_owned(),
        root.to_string_lossy().into_owned(),
        "--lcov".to_owned(),
        lcov.to_string_lossy().into_owned(),
    ];
    arguments.extend(extra.iter().map(|value| (*value).to_owned()));
    arguments.push("--json".to_owned());
    omnirepo_dev::run(arguments)
}

fn write_lcov(root: &std::path::Path, lcov: &std::path::Path, content: &str) {
    std::fs::write(lcov, content).expect("write LCOV");
    let _ = root;
}

fn fixture() -> (LifecycleFixture, PathBuf, PathBuf) {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-cli", 7_701))
        .expect("coverage fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create fixture repository");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() {\n    println!(\"changed\");\n}\n",
    )
    .expect("write changed source");
    git(&fixture, &root, &["add", "."]);
    git(
        &fixture,
        &root,
        &["commit", "--quiet", "--message", "changed"],
    );
    let lcov = root.join("coverage.info");
    std::fs::write(
        &lcov,
        "TN:\nSF:src/main.rs\nDA:2,1\nLF:1\nLH:1\nend_of_record\n",
    )
    .expect("write LCOV");
    (fixture, root, lcov)
}

#[test]
fn renamed_rust_file_maps_to_new_path_and_counts_once() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-rename", 7_702))
        .expect("rename fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(
        root.join("src").join("old.rs"),
        "fn old() {\n    println!(\"base\");\n}\n",
    )
    .expect("write old source");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    git(&fixture, &root, &["mv", "src/old.rs", "src/new.rs"]);
    std::fs::write(
        root.join("src").join("new.rs"),
        "fn new() {\n    println!(\"renamed\");\n    let uncovered = true;\n}\n",
    )
    .expect("write renamed source");
    git(&fixture, &root, &["add", "."]);
    git(
        &fixture,
        &root,
        &["commit", "--quiet", "--message", "rename"],
    );
    let lcov = root.join("coverage.info");
    write_lcov(
        &root,
        &lcov,
        "TN:\nSF:src/new.rs\nDA:1,1\nDA:2,1\nDA:3,0\nLF:3\nLH:2\nend_of_record\n",
    );
    let report_path = root.join("report.json");
    let output = run_evaluator(
        &root,
        &lcov,
        &[
            "--base",
            "HEAD~1",
            "--report",
            report_path.to_str().expect("report path"),
        ],
    );
    assert_eq!(
        output.status, 1,
        "stdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
    let report: ChangedReport = serde_json::from_str(&output.stdout).expect("valid report");
    assert_eq!(report.executable_changed_lines, 3);
    assert_eq!(report.covered_changed_lines, 2);
    assert_eq!(report.coverage_percent, Some(66));
    assert_eq!(report.coverage_ratio, "2/3");
    assert!(!report.passed);
    let persisted = std::fs::read_to_string(&report_path).expect("persisted report");
    assert!(persisted.contains("\"path\":\"src/new.rs\""));
    assert!(!persisted.contains("src/old.rs"));
}

#[test]
fn deleted_and_comment_only_changes_report_explicit_zero_sample() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-zero", 7_703))
        .expect("zero fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(
        root.join("src").join("gone.rs"),
        "fn gone() {\n    println!(\"bye\");\n}\n",
    )
    .expect("write doomed source");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    // Head deletes the Rust file, edits a comment in the manifest, and adds a
    // declaration-only module (no executable lines, so no LCOV record exists).
    git(&fixture, &root, &["rm", "--quiet", "src/gone.rs"]);
    std::fs::create_dir_all(root.join("src/decl")).expect("create decl module");
    std::fs::write(
        root.join("src/decl/mod.rs"),
        "//! Declaration-only module.\n#![allow(dead_code)]\nmod inner;\n",
    )
    .expect("write declaration-only module");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\n# comment-only change\nname = \"fixture\"\n",
    )
    .expect("write comment-only manifest");
    git(&fixture, &root, &["add", "."]);
    git(
        &fixture,
        &root,
        &["commit", "--quiet", "--message", "delete"],
    );
    let lcov = root.join("coverage.info");
    write_lcov(&root, &lcov, "TN:\n");
    let report_path = root.join("report.json");
    let output = run_evaluator(
        &root,
        &lcov,
        &[
            "--base",
            "HEAD~1",
            "--report",
            report_path.to_str().expect("report path"),
        ],
    );
    assert_eq!(
        output.status, 0,
        "stdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
    let report: ChangedReport = serde_json::from_str(&output.stdout).expect("valid report");
    assert_eq!(report.executable_changed_lines, 0);
    assert_eq!(report.covered_changed_lines, 0);
    assert_eq!(report.coverage_percent, None);
    assert_eq!(report.coverage_ratio, "0/0");
    assert!(report.passed, "zero sample passes without a 100% sample");
}

#[test]
fn unresolvable_base_fails_closed() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-base", 7_704))
        .expect("base fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    let lcov = root.join("coverage.info");
    write_lcov(&root, &lcov, "TN:\nend_of_record\n");
    // A revision outside the available history must not resolve to a sample.
    let output = run_evaluator(
        &root,
        &lcov,
        &["--base", "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"],
    );
    assert_eq!(output.status, 1);
    assert!(output.stderr.contains("git comparison failed"));
    assert!(output.stdout.is_empty());
}

#[test]
fn missing_lcov_record_for_changed_file_fails_closed() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-missing", 7_705))
        .expect("missing fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() {\n    println!(\"changed\");\n}\n",
    )
    .expect("write changed source");
    git(&fixture, &root, &["add", "."]);
    git(
        &fixture,
        &root,
        &["commit", "--quiet", "--message", "changed"],
    );
    // LCOV only records an unrelated file, never the changed source.
    let lcov = root.join("coverage.info");
    write_lcov(
        &root,
        &lcov,
        "TN:\nSF:src/other.rs\nDA:1,1\nend_of_record\n",
    );
    let output = run_evaluator(&root, &lcov, &["--base", "HEAD~1"]);
    assert_eq!(output.status, 1);
    assert!(output.stderr.contains("missing executable source record"));
}

#[test]
fn report_write_failure_fails_closed() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-write", 7_706))
        .expect("write fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() {\n    println!(\"changed\");\n}\n",
    )
    .expect("write changed source");
    git(&fixture, &root, &["add", "."]);
    git(
        &fixture,
        &root,
        &["commit", "--quiet", "--message", "changed"],
    );
    let lcov = root.join("coverage.info");
    write_lcov(&root, &lcov, "TN:\nSF:src/main.rs\nDA:2,1\nend_of_record\n");
    // The report directory does not exist; the write must fail closed.
    let output = run_evaluator(
        &root,
        &lcov,
        &[
            "--base",
            "HEAD~1",
            "--report",
            "/nonexistent-dir-for-coverage/out.json",
        ],
    );
    assert_eq!(output.status, 1);
    assert!(output.stderr.contains("cannot write"));
}

#[test]
fn threshold_failure_persists_report_and_does_not_mask() {
    let fixture = LifecycleFixture::create(FixtureSpec::new("changed-coverage-precedence", 7_707))
        .expect("precedence fixture");
    let root = fixture.roots().root().join("repository");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    git(&fixture, &root, &["init", "--quiet"]);
    git(&fixture, &root, &["config", "user.name", "Coverage"]);
    git(
        &fixture,
        &root,
        &["config", "user.email", "coverage@example.test"],
    );
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
        .expect("write manifest");
    git(&fixture, &root, &["add", "."]);
    git(&fixture, &root, &["commit", "--quiet", "--message", "base"]);
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() {\n    println!(\"changed\");\n    let uncovered = true;\n}\n",
    )
    .expect("write changed source");
    git(&fixture, &root, &["add", "."]);
    git(
        &fixture,
        &root,
        &["commit", "--quiet", "--message", "changed"],
    );
    let lcov = root.join("coverage.info");
    write_lcov(
        &root,
        &lcov,
        "TN:\nSF:src/main.rs\nDA:1,1\nDA:2,1\nDA:3,0\nend_of_record\n",
    );
    let report_path = root.join("report.json");
    let output = run_evaluator(
        &root,
        &lcov,
        &[
            "--base",
            "HEAD~1",
            "--report",
            report_path.to_str().expect("report path"),
        ],
    );
    // Threshold failure is the result even though the report was generated.
    assert_eq!(output.status, 1);
    let report: ChangedReport = serde_json::from_str(&output.stdout).expect("valid report");
    assert_eq!(report.coverage_ratio, "2/3");
    assert!(!report.passed);
    let persisted = std::fs::read_to_string(&report_path).expect("persisted report");
    assert!(persisted.contains("\"passed\":false"));
}

#[test]
fn cli_reports_changed_coverage_and_writes_diagnostics() {
    let (_fixture, root, lcov) = fixture();
    let diff = std::process::Command::new("git")
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            "HEAD~1",
            "--",
        ])
        .current_dir(&root)
        .output()
        .expect("capture fixture diff");
    assert!(
        diff.status.success()
            && String::from_utf8_lossy(&diff.stdout).contains("+++ b/src/main.rs"),
        "fixture diff failed: stdout={} stderr={}",
        String::from_utf8_lossy(&diff.stdout),
        String::from_utf8_lossy(&diff.stderr)
    );
    let report_path = root.join("changed-coverage.json");
    let output = omnirepo_dev::run([
        "changed-coverage",
        "--repo-root",
        root.to_str().expect("fixture path is UTF-8"),
        "--lcov",
        lcov.to_str().expect("LCOV path is UTF-8"),
        "--base",
        "HEAD~1",
        "--report",
        report_path.to_str().expect("report path is UTF-8"),
        "--json",
    ]);
    assert_eq!(
        output.status, 0,
        "stdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
    assert!(output.stderr.is_empty());
    let report: ChangedReport = serde_json::from_str(&output.stdout).expect("valid report");
    assert_eq!(report.schema, REPORT_SCHEMA);
    assert_eq!(
        report.executable_changed_lines,
        1,
        "report: {report:?}\ndiff: {}",
        String::from_utf8_lossy(&diff.stdout)
    );
    assert_eq!(report.coverage_percent, Some(100));
    assert_eq!(report.coverage_ratio, "1/1");
    assert!(report.passed);
    assert!(!report.base.is_empty());
    assert!(!report.head.is_empty());
    let persisted = std::fs::read_to_string(&report_path).expect("persisted report");
    assert!(persisted.contains("\"threshold_percent\":80"));
}

#[test]
fn missing_explicit_or_resolved_base_fails_closed() {
    let (_fixture, root, lcov) = fixture();
    let arguments = [
        "changed-coverage".to_owned(),
        "--repo-root".to_owned(),
        root.to_string_lossy().into_owned(),
        "--lcov".to_owned(),
        lcov.to_string_lossy().into_owned(),
        "--json".to_owned(),
    ];
    // The ambient OMNIREPO_COVERAGE_BASE contract must not rescue the missing
    // base case; the test pins the variable absent and present explicitly.
    unsafe { std::env::remove_var("OMNIREPO_COVERAGE_BASE") };
    let output = omnirepo_dev::run(arguments.clone());
    assert_eq!(output.status, 1);
    assert!(output.stderr.contains("requires --base"));

    unsafe { std::env::set_var("OMNIREPO_COVERAGE_BASE", "HEAD~1") };
    let output = omnirepo_dev::run(arguments);
    unsafe { std::env::remove_var("OMNIREPO_COVERAGE_BASE") };
    assert_eq!(output.status, 0);
    assert!(output.stderr.is_empty());
}
