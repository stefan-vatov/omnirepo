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
        coverage_percent: 100,
        passed: true,
        lines: Vec::new(),
    };
    let json = report.json().expect("bounded report");
    assert!(json.contains("\"base\":\"base-sha\""));
    assert!(json.contains("\"head\":\"head-sha\""));
    assert!(json.contains("\"threshold_percent\":95"));
}

#[derive(Debug, Deserialize)]
struct ChangedReport {
    schema: String,
    base: String,
    head: String,
    executable_changed_lines: u64,
    coverage_percent: u64,
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
        diff.status.success() && String::from_utf8_lossy(&diff.stdout).contains("+++ b/src/main.rs"),
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
    assert_eq!(report.coverage_percent, 100);
    assert!(report.passed);
    assert!(!report.base.is_empty());
    assert!(!report.head.is_empty());
    let persisted = std::fs::read_to_string(&report_path).expect("persisted report");
    assert!(persisted.contains("\"threshold_percent\":95"));
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
    let output = omnirepo_dev::run(arguments.clone());
    assert_eq!(output.status, 1);
    assert!(output.stderr.contains("requires --base"));

    unsafe { std::env::set_var("OMNIREPO_COVERAGE_BASE", "HEAD~1") };
    let output = omnirepo_dev::run(arguments);
    unsafe { std::env::remove_var("OMNIREPO_COVERAGE_BASE") };
    assert_eq!(output.status, 0);
    assert!(output.stderr.is_empty());
}
