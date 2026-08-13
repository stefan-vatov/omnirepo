//! Contract tests for deterministic coverage ownership attribution.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use omnirepo_dev::coverage::{CoverageError, attribute_lcov, attribute_repository};

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn fixture_root(name: &str) -> PathBuf {
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "omnirepo-coverage-ownership-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).expect("create source fixture");
    root
}

fn write_fixture(
    root: &Path,
    source_name: &str,
    source: &str,
    matrix: &str,
    ownership: &str,
    lcov: &str,
) {
    fs::write(root.join("src").join(source_name), source).expect("write source fixture");
    fs::write(root.join("matrix.json"), matrix).expect("write matrix fixture");
    fs::write(root.join("ownership.json"), ownership).expect("write ownership fixture");
    fs::write(root.join("lcov.info"), lcov).expect("write LCOV fixture");
}

fn matrix(row_id: &str, case_id: &str, evidence_id: &str, owner: &str) -> String {
    format!(
        r#"{{"schema":"omnirepo.traceability-matrix.v1","status":"canonical","rows":[{{"id":"{row_id}","case_id":"{case_id}","evidence_id":"{evidence_id}","primary_owner":"{owner}"}}]}}"#
    )
}

fn ownership(path: &str, row_id: &str, case_id: &str, evidence_id: &str, owner: &str) -> String {
    let _ = (case_id, evidence_id, owner);
    format!(
        r#"{{"schema":"omnirepo.coverage-ownership.v1","status":"canonical-projection","entries":[{{"path":"{path}","row_id":"{row_id}"}}]}}"#
    )
}

fn ownership_entries(paths: &[&str], row_id: &str) -> String {
    let entries = paths
        .iter()
        .map(|path| format!(r#"{{"path":"{path}","row_id":"{row_id}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"schema":"omnirepo.coverage-ownership.v1","status":"canonical-projection","entries":[{entries}]}}"#
    )
}

fn lcov(path: &str) -> String {
    format!(
        "TN:\nSF:{path}\nFN:3,uncovered\nFNDA:0,uncovered\nFNF:1\nFNH:0\nDA:3,0\nDA:4,2\nLF:2\nLH:1\nBRDA:3,0,0,-\nBRDA:4,0,1,1\nBRF:2\nBRH:1\nend_of_record\n"
    )
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

fn collect_runtime_sources(root: &Path, directory: &Path, paths: &mut Vec<String>) {
    for entry in fs::read_dir(directory).expect("read runtime source directory") {
        let entry = entry.expect("read runtime source entry");
        let path = entry.path();
        if path
            .strip_prefix(root)
            .expect("runtime source is under repository root")
            .components()
            .any(|component| component.as_os_str() == std::ffi::OsStr::new("tests"))
        {
            continue;
        }
        if path.is_dir() {
            collect_runtime_sources(root, &path, paths);
            continue;
        }
        let is_rust = path.extension().and_then(|extension| extension.to_str()) == Some("rs");
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if is_rust && name != "tests.rs" && !name.ends_with("_tests.rs") {
            paths.push(
                path.strip_prefix(root)
                    .expect("runtime source is under repository root")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

#[test]
fn valid_report_attributes_exact_uncovered_lines_functions_and_regions() {
    let root = fixture_root("valid");
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    write_fixture(
        &root,
        "main.rs",
        "fn uncovered() {}\nfn covered() {}\n",
        &matrix,
        &ownership,
        &lcov("src/main.rs"),
    );

    let report = attribute_repository(
        &root,
        &root.join("lcov.info"),
        &root.join("matrix.json"),
        &root.join("ownership.json"),
    )
    .expect("valid coverage should attribute");

    assert_eq!(report.schema, "omnirepo.coverage-ownership-report.v1");
    assert_eq!(report.scope, "publishable-product-src");
    assert_eq!(report.totals.lines_total, 2);
    assert_eq!(report.totals.lines_covered, 1);
    assert_eq!(report.totals.functions_total, 1);
    assert_eq!(report.totals.functions_covered, 0);
    assert_eq!(report.totals.regions_total, 2);
    assert_eq!(report.totals.regions_covered, 1);
    assert_eq!(report.sources[0].uncovered_lines[0].line, 3);
    assert_eq!(report.sources[0].uncovered_functions[0].name, "uncovered");
    assert_eq!(report.sources[0].uncovered_regions[0].line, 3);
    assert_eq!(report.sources[0].owner.primary_owner, "omni-fixture-1");
    assert!(
        report
            .json()
            .expect("bounded report JSON")
            .contains("behavior-fixture")
    );
    cleanup(&root);
}

#[test]
fn authoritative_function_summary_may_differ_from_detail_cardinality() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let lcov = concat!(
        "TN:\n",
        "SF:src/main.rs\n",
        "FN:3,covered\n",
        "FN:4,uncovered\n",
        "FNDA:1,covered\n",
        "FNDA:0,uncovered\n",
        "FNF:1\n",
        "FNH:1\n",
        "DA:3,1\n",
        "LF:1\n",
        "LH:1\n",
        "BRF:0\n",
        "BRH:0\n",
        "end_of_record\n",
    );

    let report = attribute_lcov(lcov, &matrix, &ownership)
        .expect("authoritative function summaries may differ from detail records");
    let source = &report.sources[0];
    assert_eq!(source.totals.functions_total, 1);
    assert_eq!(source.totals.functions_covered, 1);
    assert_eq!(source.uncovered_functions.len(), 1);
    assert_eq!(source.uncovered_functions[0].name, "uncovered");

    let json = report.json().expect("ownership JSON remains bounded");
    assert!(!json.is_empty());
    assert!(json.len() <= 1024 * 1024);
}

#[test]
fn malformed_function_details_or_summaries_fail_closed() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let expect_error = |lcov: String, expected: &str| {
        let error = attribute_lcov(&lcov, &matrix, &ownership)
            .expect_err("malformed function coverage must fail closed");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?} in {error}"
        );
    };

    expect_error(
        lcov("src/main.rs").replace("FNDA:0,uncovered\n", ""),
        "function records and summary disagree",
    );
    expect_error(
        lcov("src/main.rs").replace("FNDA:0,uncovered", "FNDA:0,other"),
        "function records and summary disagree",
    );
    expect_error(
        lcov("src/main.rs").replace("FNH:0", "FNH:2"),
        "FNH cannot exceed FNF",
    );
    expect_error(
        lcov("src/main.rs").replace("FNF:1\n", ""),
        "missing FNF summary",
    );
    expect_error(
        lcov("src/main.rs").replace("FNF:1\n", "FNF:not-a-number\n"),
        "FNF must be an unsigned integer",
    );
    expect_error(
        lcov("src/main.rs").replace("FNF:1\n", "FNF:1\nFNF:1\n"),
        "duplicate FNF summary",
    );
    expect_error(
        lcov("src/main.rs").replace("LH:1", "LH:3"),
        "LH cannot exceed LF",
    );
    expect_error(
        lcov("src/main.rs").replace("BRH:1", "BRH:3"),
        "BRH cannot exceed BRF",
    );
}

#[test]
fn non_product_workspace_records_do_not_become_product_ownership() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let report = attribute_lcov(
        &format!(
            "{}TN:dev\nSF:tools/omnirepo-dev/src/lib.rs\nDA:1,0\nLF:1\nLH:0\nend_of_record\n",
            lcov("src/main.rs")
        ),
        &matrix,
        &ownership,
    )
    .expect("development-only records are outside product scope");
    assert_eq!(report.sources.len(), 1);
    assert_eq!(report.sources[0].path, "src/main.rs");
}

#[test]
fn product_test_subtrees_do_not_become_runtime_ownership() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let test_record = "TN:\nSF:src/platform/authority/tests/coverage_tests/backend.rs\nDA:1,0\nLF:1\nLH:0\nend_of_record\n";
    let report = attribute_lcov(
        &format!("{}{}", lcov("src/main.rs"), test_record),
        &matrix,
        &ownership,
    )
    .expect("test-only source trees are outside product ownership");
    assert_eq!(report.sources.len(), 1);
    assert_eq!(report.sources[0].path, "src/main.rs");
}

#[test]
fn absolute_lcov_paths_are_normalized_to_repository_relative_paths() {
    let root = fixture_root("absolute-path");
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let lcov = lcov(&root.join("src/main.rs").to_string_lossy());
    write_fixture(
        &root,
        "main.rs",
        "fn uncovered() {}\nfn covered() {}\n",
        &matrix,
        &ownership,
        &lcov,
    );
    let report = attribute_repository(
        &root,
        &root.join("lcov.info"),
        &root.join("matrix.json"),
        &root.join("ownership.json"),
    )
    .expect("absolute LCOV path should normalize");
    assert_eq!(report.sources[0].path, "src/main.rs");
    cleanup(&root);
}

#[test]
fn unmapped_product_source_fails_closed() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let error = attribute_lcov(&lcov("src/unknown.rs"), &matrix, &ownership)
        .expect_err("unmapped product source must fail");
    assert!(matches!(error, CoverageError::Ownership { .. }));
    assert!(error.to_string().contains("src/unknown.rs"));
}

#[test]
fn matrix_identity_mismatch_fails_closed() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "unknown-row",
        "trace.behavior.other",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let error = attribute_lcov(&lcov("src/main.rs"), &matrix, &ownership)
        .expect_err("identity drift must fail");
    assert!(
        error
            .to_string()
            .contains("absent from the canonical matrix")
    );
}

#[test]
fn duplicate_json_keys_fail_closed() {
    let matrix = r#"{"schema":"omnirepo.traceability-matrix.v1","schema":"omnirepo.traceability-matrix.v1","status":"canonical","rows":[]}"#;
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let error = attribute_lcov(&lcov("src/main.rs"), matrix, &ownership)
        .expect_err("duplicate keys must fail");
    assert!(error.to_string().contains("duplicate object key"));
}

#[test]
fn malformed_lcov_requires_complete_records() {
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership(
        "src/main.rs",
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let error = attribute_lcov(
        "TN:fixture\nSF:src/main.rs\nDA:1,0\nLF:1\nLH:0\n",
        &matrix,
        &ownership,
    )
    .expect_err("truncated LCOV must fail");
    assert!(error.to_string().contains("missing end_of_record"));
}

#[test]
fn checked_in_projection_matches_current_runtime_source_tree() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("developer-tool crate is nested under repository root");
    let mut paths = Vec::new();
    collect_runtime_sources(repository_root, &repository_root.join("src"), &mut paths);
    paths.sort();
    let lcov = paths
        .iter()
        .map(|path| format!("TN:\nSF:{path}\nDA:1,1\nLF:1\nLH:1\nend_of_record\n"))
        .collect::<String>();
    let lcov_path = std::env::temp_dir().join(format!(
        "omnirepo-coverage-projection-{}-{}.info",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&lcov_path, lcov).expect("write projection LCOV fixture");

    let report = attribute_repository(
        repository_root,
        &lcov_path,
        &repository_root.join("tests/traceability/matrix.json"),
        &repository_root.join("tests/traceability/coverage-ownership.json"),
    )
    .expect("projection must cover every current runtime source");

    assert_eq!(report.sources.len(), paths.len());
    assert_eq!(report.totals.lines_total, paths.len() as u64);
    let _ = fs::remove_file(lcov_path);
}

#[test]
fn sources_without_executable_lcov_records_remain_in_bounded_report() {
    let root = fixture_root("no-executable-record");
    fs::write(
        root.join("src/mod.rs"),
        r#"// A declaration-only facade.
#![allow(dead_code)]
#[path = "child.rs"]
pub(crate) mod child;

/* A multiline re-export remains declaration-only. */
#[cfg(feature = "x")]
pub use self::{
    child,
};
"#,
    )
    .expect("write declaration-only source");
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = r#"{
        "schema":"omnirepo.coverage-ownership.v1",
        "status":"canonical-projection",
        "entries":[
            {"path":"src/main.rs","row_id":"behavior-fixture"},
            {"path":"src/mod.rs","row_id":"behavior-fixture"}
        ]
    }"#;
    write_fixture(
        &root,
        "main.rs",
        "fn covered() {}\n",
        &matrix,
        ownership,
        &lcov("src/main.rs"),
    );

    let report = attribute_repository(
        &root,
        &root.join("lcov.info"),
        &root.join("matrix.json"),
        &root.join("ownership.json"),
    )
    .expect("declaration-only source may have no LCOV record");
    let source = report
        .sources
        .iter()
        .find(|source| source.path == "src/mod.rs")
        .expect("declaration-only source remains mapped");
    assert_eq!(source.totals, Default::default());
    let json = report.json().expect("ownership JSON remains bounded");
    assert!(!json.is_empty());
    assert!(json.len() <= 1024 * 1024);
    cleanup(&root);
}

#[test]
fn missing_lcov_record_for_executable_source_fails_closed() {
    let root = fixture_root("missing-executable-record");
    fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write executable source");
    fs::write(root.join("src/anchor.rs"), "mod child;\n").expect("write LCOV anchor source");
    let matrix = matrix(
        "behavior-fixture",
        "trace.behavior.fixture",
        "evidence.trace.fixture.v1",
        "omni-fixture-1",
    );
    let ownership = ownership_entries(&["src/anchor.rs", "src/main.rs"], "behavior-fixture");
    fs::write(root.join("matrix.json"), matrix).expect("write matrix fixture");
    fs::write(root.join("ownership.json"), ownership).expect("write ownership fixture");
    fs::write(root.join("lcov.info"), lcov("src/anchor.rs")).expect("write LCOV anchor record");
    let error = attribute_repository(
        &root,
        &root.join("lcov.info"),
        &root.join("matrix.json"),
        &root.join("ownership.json"),
    )
    .expect_err("an executable source without an LCOV record must fail closed");
    assert!(error.to_string().contains("src/main.rs"));
    assert!(error.to_string().contains("no LCOV record"));
    cleanup(&root);
}

#[test]
fn missing_lcov_record_for_non_facade_source_fails_closed() {
    let cases = [
        ("function", "fn main() {}\n"),
        ("const", "const VALUE: u8 = 1;\n"),
        ("static", "static VALUE: u8 = 1;\n"),
        ("inline-mod", "mod child { pub fn main() {} }\n"),
        ("macro", "macro_rules! generated { () => {}; }\n"),
    ];
    for (name, source) in cases {
        let root = fixture_root(&format!("missing-non-facade-{name}"));
        fs::write(root.join("src/main.rs"), source).expect("write non-facade source");
        fs::write(root.join("src/anchor.rs"), "mod child;\n").expect("write LCOV anchor source");
        fs::write(
            root.join("matrix.json"),
            matrix(
                "behavior-fixture",
                "trace.behavior.fixture",
                "evidence.trace.fixture.v1",
                "omni-fixture-1",
            ),
        )
        .expect("write matrix fixture");
        fs::write(
            root.join("ownership.json"),
            ownership_entries(&["src/anchor.rs", "src/main.rs"], "behavior-fixture"),
        )
        .expect("write ownership fixture");
        fs::write(root.join("lcov.info"), lcov("src/anchor.rs")).expect("write LCOV anchor record");

        let error = attribute_repository(
            &root,
            &root.join("lcov.info"),
            &root.join("matrix.json"),
            &root.join("ownership.json"),
        )
        .expect_err("non-facade source without an LCOV record must fail closed");
        assert!(error.to_string().contains("src/main.rs"));
        assert!(error.to_string().contains("no LCOV record"));
        cleanup(&root);
    }
}
